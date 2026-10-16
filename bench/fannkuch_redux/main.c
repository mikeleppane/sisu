#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static void fannkuch(int64_t n) {
    int64_t *perm1 = malloc(n * sizeof *perm1);
    int64_t *perm = malloc(n * sizeof *perm);
    int64_t *count = malloc(n * sizeof *count);
    if (!perm1 || !perm || !count) abort();
    for (int64_t i = 0; i < n; i++) perm1[i] = i;
    int64_t max_flips = 0, checksum = 0, perm_count = 0, r = n;
    for (;;) {
        while (r != 1) {
            count[r - 1] = r;
            r--;
        }
        for (int64_t i = 0; i < n; i++) perm[i] = perm1[i];
        int64_t flips = 0;
        int64_t k = perm[0];
        while (k != 0) {
            for (int64_t i = 0, j = k; i < j; i++, j--) {
                int64_t t = perm[i];
                perm[i] = perm[j];
                perm[j] = t;
            }
            flips++;
            k = perm[0];
        }
        if (flips > max_flips) max_flips = flips;
        checksum += perm_count % 2 == 0 ? flips : -flips;
        for (;;) {
            if (r == n) {
                printf("%ld\nPfannkuchen(%ld) = %ld\n", checksum, n, max_flips);
                free(perm1);
                free(perm);
                free(count);
                return;
            }
            int64_t first = perm1[0];
            for (int64_t i = 0; i < r; i++) perm1[i] = perm1[i + 1];
            perm1[r] = first;
            count[r]--;
            if (count[r] > 0) break;
            r++;
        }
        perm_count++;
    }
}

int main(void) {
    fannkuch(10);
    return 0;
}
