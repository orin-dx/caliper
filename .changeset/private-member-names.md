---
cargo/caliper-docgen: patch
npm/caliper-docgen: patch
---

**An ES-private class member keeps the name you wrote, not the checker's internal name.** A field written `#type` now shows as `#type` instead of `__#8@#type`.

- Why it matters: the number was the checker's symbol id and changed with the order it was asked, so the same project could give two different names from run to run.
- Limits: this changes the name of private members that reach a prop's type (for example through a class from a dependency, such as `CalendarDate`). Nothing else changes.
