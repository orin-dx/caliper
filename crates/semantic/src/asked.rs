//! The checker's answers kept for one snapshot, each asked once however many conversions want it, at once or later.
//!
//! The lanes of a run answer several components at a time over one converter ([`crate::convert::Converter`]), and
//! components of one library walk the same types in the same order: without this, two of them asked every question
//! twice, each missing a memo the other had not filled yet (Chakra: 520k of 732k `isReadonlySymbol` requests, 5,400 of
//! 8,900 `typeToString`). An answer is a function of the snapshot and the question alone, so which conversion asks it
//! first cannot show in the output.

use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use rustc_hash::FxHashMap;
use tokio::sync::{Mutex as AsyncMutex, OnceCell, OwnedMutexGuard};

use crate::rpc::CallError;

/// Answers by question. A question asked while another conversion waits for its answer waits for that answer instead
/// of asking again. An answer that failed is not kept: the next to ask asks again, as it would have without this.
pub(crate) struct Answers<K, V> {
    cells: Mutex<FxHashMap<K, Arc<OnceCell<V>>>>,
}

impl<K, V> Default for Answers<K, V> {
    fn default() -> Self {
        Answers { cells: Mutex::default() }
    }
}

impl<K: Eq + Hash, V: Clone> Answers<K, V> {
    /// The answer to `key`, from `ask` when it was not asked before.
    pub(crate) async fn get<F, Fut>(&self, key: K, ask: F) -> Result<V, CallError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, CallError>>,
    {
        let cell = self.cells.lock().unwrap_or_else(PoisonError::into_inner).entry(key).or_default().clone();
        cell.get_or_try_init(ask).await.cloned()
    }
}

/// Answers that go out in a batch with others: what [`Answers`] keeps for a question asked alone. A caller takes each
/// question it needs ([`BatchAnswers::take`]), sends the ones it was given as one request, answers them, and only then
/// waits for the ones another caller was given, so a wait is never for an answer that waits on one of ours. A question
/// whose asker failed is not kept: whoever waited for it takes it on the next round.
pub(crate) struct BatchAnswers<K, V> {
    slots: Mutex<FxHashMap<K, Arc<Slot<V>>>>,
}

struct Slot<V> {
    answer: OnceLock<V>,
    /// Held by the one caller asking this question.
    asking: Arc<AsyncMutex<()>>,
}

/// What [`BatchAnswers::take`] says about a question.
pub(crate) enum Taken<V> {
    /// Answered: [`BatchAnswers::peek`] has it.
    Kept,
    /// Nobody is asking it, and the caller now is: it must [`Mine::answer`] it, or drop it to leave it unanswered.
    Mine(Mine<V>),
    /// Another caller is asking it.
    Theirs(Theirs),
}

/// A question the caller is asking.
pub(crate) struct Mine<V> {
    slot: Arc<Slot<V>>,
    _asking: OwnedMutexGuard<()>,
}

/// A question another caller is asking.
pub(crate) struct Theirs(Arc<AsyncMutex<()>>);

impl<K, V> Default for BatchAnswers<K, V> {
    fn default() -> Self {
        BatchAnswers { slots: Mutex::default() }
    }
}

impl<K: Eq + Hash, V: Clone> BatchAnswers<K, V> {
    /// The caller's part in the question `key`.
    pub(crate) fn take(&self, key: K) -> Taken<V> {
        let slot = self
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key)
            .or_insert_with(|| Arc::new(Slot { answer: OnceLock::new(), asking: Arc::new(AsyncMutex::new(())) }))
            .clone();
        if slot.answer.get().is_some() {
            return Taken::Kept;
        }
        match slot.asking.clone().try_lock_owned() {
            // Answered between the two looks.
            Ok(_) if slot.answer.get().is_some() => Taken::Kept,
            Ok(asking) => Taken::Mine(Mine { slot, _asking: asking }),
            Err(_) => Taken::Theirs(Theirs(slot.asking.clone())),
        }
    }

    /// The answer to `key`, when it has one.
    pub(crate) fn peek(&self, key: &K) -> Option<V> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner).get(key)?.answer.get().cloned()
    }
}

impl<V> Mine<V> {
    pub(crate) fn answer(self, answer: V) {
        self.slot.answer.set(answer).ok();
    }
}

impl Theirs {
    /// Waits until the asker has answered or given up: take the question again to know which.
    pub(crate) async fn wait(self) {
        drop(self.0.lock().await);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    fn run<T>(work: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(work)
    }

    #[test]
    fn a_question_is_asked_once_and_a_failure_is_asked_again() {
        run(async {
            let answers: Answers<u64, u32> = Answers::default();
            let asked = AtomicU32::new(0);
            let ask = |value: u32| {
                let asked = &asked;
                move || async move {
                    asked.fetch_add(1, Ordering::Relaxed);
                    Ok(value)
                }
            };
            assert_eq!(answers.get(1, ask(10)).await.unwrap(), 10);
            assert_eq!(answers.get(1, ask(20)).await.unwrap(), 10);
            assert_eq!(answers.get(2, ask(30)).await.unwrap(), 30);
            assert_eq!(asked.load(Ordering::Relaxed), 2);

            let failed = answers.get(3, || async { Err(CallError::Rejected("no".to_string())) }).await;
            assert!(matches!(failed, Err(CallError::Rejected(message)) if message == "no"));
            assert_eq!(answers.get(3, ask(40)).await.unwrap(), 40);
            assert_eq!(asked.load(Ordering::Relaxed), 3);
        });
    }

    #[test]
    fn a_question_asked_while_it_is_being_answered_waits_for_that_answer() {
        run(async {
            let answers: Arc<Answers<u64, u32>> = Arc::default();
            let asked = Arc::new(AtomicU32::new(0));
            let (release, wait) = tokio::sync::oneshot::channel::<()>();
            let first = tokio::spawn({
                let (answers, asked) = (answers.clone(), asked.clone());
                async move {
                    answers
                        .get(1, || async move {
                            asked.fetch_add(1, Ordering::Relaxed);
                            wait.await.ok();
                            Ok(1)
                        })
                        .await
                }
            });
            // The first is now waiting on its answer.
            tokio::task::yield_now().await;
            let second = tokio::spawn({
                let (answers, asked) = (answers.clone(), asked.clone());
                async move {
                    answers
                        .get(1, || async move {
                            asked.fetch_add(1, Ordering::Relaxed);
                            Ok(2)
                        })
                        .await
                }
            });
            tokio::task::yield_now().await;
            release.send(()).unwrap();
            assert_eq!((first.await.unwrap().unwrap(), second.await.unwrap().unwrap()), (1, 1));
            assert_eq!(asked.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn a_question_another_caller_is_asking_is_theirs_until_it_is_answered() {
        run(async {
            let answers: BatchAnswers<u64, u32> = BatchAnswers::default();
            let Taken::Mine(mine) = answers.take(1) else { panic!("nobody was asking it") };
            let (Taken::Theirs(theirs), Taken::Mine(other)) = (answers.take(1), answers.take(2)) else {
                panic!("one is being asked, the other is not")
            };
            assert_eq!(answers.peek(&1), None);

            mine.answer(10);
            theirs.wait().await;
            other.answer(20);

            assert!(matches!(answers.take(1), Taken::Kept) && matches!(answers.take(2), Taken::Kept));
            assert_eq!((answers.peek(&1), answers.peek(&2)), (Some(10), Some(20)));
        });
    }

    #[test]
    fn a_question_its_asker_gave_up_on_is_taken_by_the_next_caller() {
        run(async {
            let answers: Arc<BatchAnswers<u64, u32>> = Arc::default();
            let Taken::Mine(mine) = answers.take(1) else { panic!("nobody was asking it") };
            let waiter = tokio::spawn({
                let answers = answers.clone();
                async move {
                    let Taken::Theirs(theirs) = answers.take(1) else { panic!("it is being asked") };
                    theirs.wait().await;
                    // Not answered: the waiter asks.
                    let Taken::Mine(mine) = answers.take(1) else { panic!("the asker gave up") };
                    mine.answer(2);
                }
            });
            tokio::task::yield_now().await;
            drop(mine);
            waiter.await.unwrap();

            assert_eq!(answers.peek(&1), Some(2));
        });
    }
}
