#include "ralloc.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#if defined(_WIN32)
#define RALLOC_HAS_THREADS 0
#else
#include <pthread.h>
#define RALLOC_HAS_THREADS 1
#endif

#define CHECK(condition)                                                        \
    do {                                                                       \
        if (!(condition)) {                                                     \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__,   \
                    #condition);                                                \
            return 1;                                                          \
        }                                                                      \
    } while (0)

static int test_zero_size_malloc_returns_null(void) {
    CHECK(ralloc_malloc(0) == NULL);
    CHECK(ralloc_calloc(0, 128) == NULL);
    CHECK(ralloc_calloc(128, 0) == NULL);
    return 0;
}

static int test_malloc_returns_aligned_writable_storage(void) {
    unsigned char *ptr = (unsigned char *)ralloc_malloc(37);
    CHECK(ptr != NULL);
    CHECK(((uintptr_t)ptr % (sizeof(size_t) * 2)) == 0);

    for (size_t i = 0; i < 37; i++) {
        ptr[i] = (unsigned char)(i + 17);
    }
    for (size_t i = 0; i < 37; i++) {
        CHECK(ptr[i] == (unsigned char)(i + 17));
    }

    ralloc_free(ptr);
    return 0;
}

static int test_aligned_alloc_alignment_validation_and_free(void) {
    unsigned char *ptr = (unsigned char *)ralloc_aligned_alloc(64, 96);
    CHECK(ptr != NULL);
    CHECK(((uintptr_t)ptr % 64) == 0);

    for (size_t i = 0; i < 96; i++) {
        ptr[i] = (unsigned char)(0xc0u ^ i);
    }
    for (size_t i = 0; i < 96; i++) {
        CHECK(ptr[i] == (unsigned char)(0xc0u ^ i));
    }
    ralloc_free(ptr);

    CHECK(ralloc_aligned_alloc(3, 96) == NULL);
    CHECK(ralloc_aligned_alloc(8192, 96) == NULL);
    CHECK(ralloc_aligned_alloc(64, (size_t)-1) == NULL);
    CHECK(ralloc_aligned_alloc(64, 0) == NULL);

    unsigned char *after_failure = (unsigned char *)ralloc_aligned_alloc(128, 64);
    CHECK(after_failure != NULL);
    CHECK(((uintptr_t)after_failure % 128) == 0);
    ralloc_free(after_failure);

    return 0;
}

static int test_free_reuses_storage_for_same_size(void) {
    void *first = ralloc_malloc(64);
    CHECK(first != NULL);
    ralloc_free(first);

    void *second = ralloc_malloc(64);
    CHECK(second == first);
    ralloc_free(second);
    return 0;
}

static int test_calloc_zeroes_and_rejects_overflow(void) {
    unsigned char *ptr = (unsigned char *)ralloc_calloc(16, 4);
    CHECK(ptr != NULL);
    for (size_t i = 0; i < 64; i++) {
        CHECK(ptr[i] == 0);
    }
    ralloc_free(ptr);

    CHECK(ralloc_calloc((size_t)-1, 2) == NULL);
    return 0;
}

static int test_realloc_null_grow_shrink_and_zero_size(void) {
    unsigned char *ptr = (unsigned char *)ralloc_realloc(NULL, 16);
    CHECK(ptr != NULL);
    for (size_t i = 0; i < 16; i++) {
        ptr[i] = (unsigned char)(0xa0u + i);
    }

    unsigned char *grown = (unsigned char *)ralloc_realloc(ptr, 128);
    CHECK(grown != NULL);
    for (size_t i = 0; i < 16; i++) {
        CHECK(grown[i] == (unsigned char)(0xa0u + i));
    }

    unsigned char *shrunk = (unsigned char *)ralloc_realloc(grown, 8);
    CHECK(shrunk != NULL);
    for (size_t i = 0; i < 8; i++) {
        CHECK(shrunk[i] == (unsigned char)(0xa0u + i));
    }

    CHECK(ralloc_realloc(shrunk, 0) == NULL);
    return 0;
}

static int test_realloc_failure_preserves_original_allocation(void) {
    unsigned char *ptr = (unsigned char *)ralloc_malloc(32);
    CHECK(ptr != NULL);
    memset(ptr, 0x5a, 32);

    CHECK(ralloc_realloc(ptr, (size_t)-1) == NULL);
    for (size_t i = 0; i < 32; i++) {
        CHECK(ptr[i] == 0x5a);
    }

    ralloc_free(ptr);
    return 0;
}

static int test_foreign_and_interior_pointers_are_rejected(void) {
    int stack_value = 7;
    ralloc_free(&stack_value);
    CHECK(ralloc_realloc(&stack_value, 64) == NULL);

    unsigned char *ptr = (unsigned char *)ralloc_malloc(64);
    CHECK(ptr != NULL);
    ptr[0] = 0x31;
    ptr[63] = 0x7e;

    ralloc_free(ptr + 1);
    CHECK(ralloc_realloc(ptr + 1, 128) == NULL);
    CHECK(ptr[0] == 0x31);
    CHECK(ptr[63] == 0x7e);

    ralloc_free(ptr);
    return 0;
}

static int test_fragmentation_coalesces_adjacent_frees(void) {
    void *first = ralloc_malloc(96);
    void *second = ralloc_malloc(96);
    void *third = ralloc_malloc(96);
    CHECK(first != NULL);
    CHECK(second != NULL);
    CHECK(third != NULL);

    ralloc_free(first);
    ralloc_free(second);

    void *combined = ralloc_malloc(176);
    CHECK(combined == first);

    ralloc_free(combined);
    ralloc_free(third);
    return 0;
}

static int test_large_allocations_spill_to_additional_arenas(void) {
    void *first = ralloc_malloc(40 * 1024);
    void *second = ralloc_malloc(40 * 1024);
    CHECK(first != NULL);
    CHECK(second != NULL);
    CHECK(first != second);

    ralloc_free(first);
    ralloc_free(second);
    return 0;
}

static int test_out_of_memory_reports_null_then_recovers_after_free(void) {
    enum { MAX_ALLOCS = 128 };
    void *allocations[MAX_ALLOCS];
    size_t count = 0;

    while (count < MAX_ALLOCS) {
        void *ptr = ralloc_malloc(8 * 1024);
        if (ptr == NULL) {
            break;
        }
        allocations[count++] = ptr;
    }

    CHECK(count > 0);
    CHECK(count < MAX_ALLOCS);
    CHECK(ralloc_malloc(8 * 1024) == NULL);

    for (size_t i = 0; i < count; i++) {
        ralloc_free(allocations[i]);
    }

    void *after_free = ralloc_malloc(8 * 1024);
    CHECK(after_free != NULL);
    ralloc_free(after_free);
    return 0;
}

#if RALLOC_HAS_THREADS
static void *thread_worker(void *arg) {
    (void)arg;
    for (size_t iteration = 0; iteration < 256; iteration++) {
        unsigned char *ptr = (unsigned char *)ralloc_malloc(128);
        if (ptr == NULL) {
            return (void *)1;
        }
        ptr[0] = (unsigned char)iteration;
        ptr[127] = (unsigned char)(iteration ^ 0xffu);
        if (ptr[0] != (unsigned char)iteration ||
            ptr[127] != (unsigned char)(iteration ^ 0xffu)) {
            return (void *)1;
        }
        ralloc_free(ptr);
    }

    return NULL;
}

static int test_concurrent_alloc_free_smoke(void) {
    pthread_t threads[4];

    for (size_t i = 0; i < 4; i++) {
        CHECK(pthread_create(&threads[i], NULL, thread_worker, NULL) == 0);
    }

    for (size_t i = 0; i < 4; i++) {
        void *result = NULL;
        CHECK(pthread_join(threads[i], &result) == 0);
        CHECK(result == NULL);
    }

    return 0;
}
#endif

struct test_case {
    const char *name;
    int (*run)(void);
};

int main(void) {
    const struct test_case tests[] = {
        {"zero-size allocation returns null", test_zero_size_malloc_returns_null},
        {"malloc returns aligned writable storage",
         test_malloc_returns_aligned_writable_storage},
        {"aligned alloc validates alignment and frees",
         test_aligned_alloc_alignment_validation_and_free},
        {"free reuses storage for same size", test_free_reuses_storage_for_same_size},
        {"calloc zeroes and rejects overflow", test_calloc_zeroes_and_rejects_overflow},
        {"realloc null/grow/shrink/zero semantics",
         test_realloc_null_grow_shrink_and_zero_size},
        {"realloc failure preserves original allocation",
         test_realloc_failure_preserves_original_allocation},
        {"foreign and interior pointers are rejected",
         test_foreign_and_interior_pointers_are_rejected},
        {"fragmentation coalesces adjacent frees",
         test_fragmentation_coalesces_adjacent_frees},
        {"large allocations spill to additional arenas",
         test_large_allocations_spill_to_additional_arenas},
        {"out-of-memory reports null then recovers",
         test_out_of_memory_reports_null_then_recovers_after_free},
#if RALLOC_HAS_THREADS
        {"concurrent alloc/free smoke", test_concurrent_alloc_free_smoke},
#endif
    };

    for (size_t i = 0; i < sizeof(tests) / sizeof(tests[0]); i++) {
        if (tests[i].run() != 0) {
            fprintf(stderr, "FAILED: %s\n", tests[i].name);
            return 1;
        }
        printf("ok - %s\n", tests[i].name);
    }

    printf("Ralloc C suite passed\n");
    return 0;
}
