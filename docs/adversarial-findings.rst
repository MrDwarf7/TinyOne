Adversarial Findings
====================

Probe 1 (FFI nulls): PASS
-------------------------

Both ``ffi_null_pointers_return_valid_json_errors`` and
``ffi_free_string_accepts_null`` passed. An all-nulls call to
``tinyone_run_source_json`` returns
``{"ok":false,"kind":"compile","error":"source pointer was null"}``, as
confirmed by test source lines 256-261. ``tinyone_free_string(null)`` is a
no-op with no crash.

Probe 2 (Hostile artifacts): PASS
---------------------------------

Both ``artifact_rejects_huge_counts_before_accepting_program`` and
``artifact_rejects_invalid_integer_fields_and_file_size`` passed. A
null-instruction artifact (a ``code`` array containing ``null``) is covered by
the code-limit path: when ``artifact["code"]`` is set to more than
``MAX_ARTIFACT_CODE_OPS`` (65,536) nulls, the count check fires before any
individual instruction is parsed. For a null-instruction artifact under the
count limit, deserialization of each instruction object produces
``Instruction artifact must be an object``. Test line 309 exercises a missing
``arg`` field with the error needle ``instruction arg``.

Probe 3 (Verifier stress): PASS
-------------------------------

All three verifier tests passed:
``verifier_handles_dense_jump_graph_without_path_explosion``,
``verifier_rejects_oversized_dense_jump_graph_before_stress``, and
``verifier_rejects_stack_depth_bomb``. The ``visit`` function in
``verifier.rs`` (lines 274-305) records the first ``pc -> stack_depth`` pair
in ``seen: HashMap<usize, i64>``. For ``[PushInt(0), JumpIfZero(0), Halt]``,
PC0 is first visited with depth 0; the jump at PC1 consumes the condition and
targets PC0 with depth 0. The matching entry in ``seen`` is accepted without
re-queuing. A different depth is rejected immediately as a stack-depth
mismatch. The guard safely terminates backward-jump loops.

Probe 4 (FS budget): PASS
-------------------------

Both ``fs_read_rejects_oversized_file_before_buffer_allocation`` and
``fs_list_dir_limit_returns_error_not_panic`` passed. The byte-budget limit,
not the count limit, triggers. With 5,000 files each named ``"00000_"`` plus
249 ``x`` characters, the total is 1,275,000 bytes, greater than
``MAX_BUFFER_BYTES`` (1,048,576). The count remains below
``MAX_FS_LIST_DIR_ENTRIES`` (65,536). The byte-budget check in ``stdlib.rs``
fires at about entry 4,113 and returns an error containing ``limit``.

Probe 5 (JIT invalid): PASS
---------------------------

``jit_compile_rejects_invalid_unverified_program`` passed.
``JitProgram::compile`` and ``JitCache::compile`` both reject the invalid
program with an error containing ``Verifier``, and the cache remains empty.
``verify_chunk`` checks that the final instruction is ``HALT``. For empty
code, ``.last()`` returns ``None``, so verification returns
``Verifier: main must end with HALT, got nothing`` before instruction
iteration begins.

Probe 6 (Char index): PASS
--------------------------

``invalid_char_index_path_returns_error_not_panic`` passed. Calling
``str_char_at("a", 9223372036854775807)`` returns an error containing
``index`` rather than panicking.

Probe 7 (Invalid mode): PASS
----------------------------

``ffi_invalid_mode_returns_structured_error`` passed. The response has
``kind == "runtime"`` and an error message containing ``Unsupported mode``,
as confirmed by test source lines 284-293.

Probe 8 (Invalid UTF-8): PASS
-----------------------------

``ffi_invalid_unicode_scalar_source_returns_error`` passed. Passing bytes
``[0xED, 0xA0, 0x80]`` (a lone surrogate, invalid UTF-8) to
``tinyone_lex_source_json`` returns a compile error that says the input must
be UTF-8.

Exploits found
--------------

None.
