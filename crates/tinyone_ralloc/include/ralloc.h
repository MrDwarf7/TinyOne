#ifndef RALLOC_H
#define RALLOC_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Ralloc exposes a narrow malloc-family C ABI. Returned storage from
 * ralloc_malloc, ralloc_calloc, and ralloc_realloc is aligned for ordinary C
 * object use. Larger alignments must use ralloc_aligned_alloc.
 *
 * Raw C pointers remain an unsafe compatibility surface: use-after-free,
 * double-free, stale-pointer reuse, and concurrent unsynchronized mutation of
 * the same allocation are caller bugs. Ralloc rejects detectably foreign or
 * interior pointers where possible and otherwise avoids promising memory safety
 * for invalid C usage.
 */

/*
 * Allocates size bytes. A zero-size request returns NULL. Out-of-memory and
 * unsupported huge requests return NULL.
 */
void *ralloc_malloc(size_t size);

/*
 * Allocates size bytes with at least alignment-byte alignment. Alignment must
 * be a nonzero power of two and no larger than 4096 bytes. A zero-size request,
 * invalid alignment, unsupported huge request, or out-of-memory condition
 * returns NULL. Successful allocations are released with ralloc_free.
 */
void *ralloc_aligned_alloc(size_t alignment, size_t size);

/*
 * Frees a pointer returned by Ralloc. NULL is ignored. Foreign, interior,
 * stale, or double-freed pointers are invalid caller input; Ralloc rejects
 * detectably invalid pointers but does not make raw C misuse memory-safe.
 */
void ralloc_free(void *ptr);

/*
 * Allocates nmemb * size zeroed bytes. Zero-size requests and multiplication
 * overflow return NULL. Out-of-memory returns NULL.
 */
void *ralloc_calloc(size_t nmemb, size_t size);

/*
 * Realloc preserves the normal malloc-family alignment contract only. Callers
 * that require a larger alignment must allocate through ralloc_aligned_alloc or
 * the Rust native aligned handle API. NULL behaves like ralloc_malloc(size).
 * size == 0 frees ptr and returns NULL. If resizing fails, the original
 * allocation remains owned by the caller.
 */
void *ralloc_realloc(void *ptr, size_t size);

#ifdef __cplusplus
}
#endif

#endif
