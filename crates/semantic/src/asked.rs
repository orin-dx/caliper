//! The checker's answers kept for one snapshot, each asked once however many conversions want it, at once or later.
//!
//! The lanes of a run answer several components at a time over one converter ([`crate::convert::Converter`]), and
//! components of one library walk the same types in the same order: without this, two of them asked every question
//! twice, each missing a memo the other had not filled yet (Chakra: 520k of 732k `isReadonlySymbol` requests, 5,400 of
//! 8,900 `typeToString`). An answer is a function of the snapshot and the question alone, so which conversion asks it
//! first cannot show in the output.

use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use rustc_hash::FxHashMap;
use tokio::sync::OnceCell;

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
}
