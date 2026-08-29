ABI Reference
=============

TinyOne's C ABI is the interface between the compiled ``libtinyone`` shared
library and any host application. It covers the JSON response contract, the
panic boundary, ownership rules, and the versioning policy.

**ABI version 1 is frozen for the entire v1 lifecycle.** TinyLang v2 is
expected to make major ABI changes as the language boundary is redesigned. See
`versioning.md <versioning.md>`_ for the compatibility policy.

Send ABI comments, concerns, and questions to the `TinyLang community
forum <https://tl.404connernotfound.dev>`_.

For how to link and call the library from C, see
`ffi/c-integration.md <../ffi/c-integration.md>`_.

Documents in This Area
----------------------

* `contract.md <contract.md>`_ -- Runtime invariants callers can rely on
  today: panic boundary, null safety, ownership, thread safety, and the
  verification guarantee.
* `ABI versioning reference <versioning.md>`_ -- What constitutes a breaking change,
  current stability status per area, and the v1 stability declaration plan.
* `schemas.md <schemas.md>`_ -- Exact JSON ``value`` schemas for every entry
  point's success response.
