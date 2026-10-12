/* Checks that every sisu_free gets the size and alignment its block was
 * allocated with. Neither ASan nor Valgrind can: the runtime frees through
 * Rust's System allocator, which calls plain free(). Link it with
 * -Wl,--wrap=sisu_alloc,--wrap=sisu_free (see toolbox.md, "Memory"). */
#include <stdio.h>
#include <stdlib.h>

void *__real_sisu_alloc(size_t size, size_t align);
void __real_sisu_free(void *ptr, size_t size, size_t align);

enum { MAX_LIVE = 1 << 20 };
static struct { void *ptr; size_t size, align; } live[MAX_LIVE];
static size_t count;

void *__wrap_sisu_alloc(size_t size, size_t align) {
    void *ptr = __real_sisu_alloc(size, align);
    if (count == MAX_LIVE) {
        fprintf(stderr, "layout-shim: more than %d live blocks\n", MAX_LIVE);
        abort();
    }
    live[count].ptr = ptr;
    live[count].size = size;
    live[count].align = align;
    count++;
    return ptr;
}

void __wrap_sisu_free(void *ptr, size_t size, size_t align) {
    for (size_t i = count; i-- > 0;) {
        if (live[i].ptr != ptr)
            continue;
        if (live[i].size != size || live[i].align != align) {
            fprintf(stderr, "layout-shim: allocated %zu/%zu, freed as %zu/%zu\n",
                    live[i].size, live[i].align, size, align);
            abort();
        }
        live[i] = live[--count];
        __real_sisu_free(ptr, size, align);
        return;
    }
    fprintf(stderr, "layout-shim: free of a block sisu_alloc did not return: %p\n", ptr);
    abort();
}
