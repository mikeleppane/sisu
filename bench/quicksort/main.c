#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static int64_t *random_array(int64_t n) {
    int64_t *xs = malloc(n * sizeof *xs);
    if (!xs) abort();
    int64_t x = 42;
    for (int64_t i = 0; i < n; i++) {
        x = (x * 1103515245 + 12345) % 2147483648;
        xs[i] = x;
    }
    return xs;
}

static void quicksort(int64_t *a, int64_t lo, int64_t hi) {
    if (lo >= hi) return;
    int64_t pivot = a[hi];
    int64_t i = lo;
    for (int64_t j = lo; j < hi; j++) {
        if (a[j] < pivot) {
            int64_t t = a[i];
            a[i] = a[j];
            a[j] = t;
            i++;
        }
    }
    a[hi] = a[i];
    a[i] = pivot;
    quicksort(a, lo, i - 1);
    quicksort(a, i + 1, hi);
}

int main(void) {
    int64_t n = 10000000;
    int64_t *xs = random_array(n);
    quicksort(xs, 0, n - 1);
    int64_t sum = 0;
    for (int64_t i = 0; i < n; i++) sum += xs[i] * (i % 100);
    free(xs);
    printf("%ld\n", sum);
    return 0;
}
