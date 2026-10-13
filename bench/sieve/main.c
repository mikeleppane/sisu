#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static bool *sieve(int64_t n) {
    bool *is_prime = malloc(n + 1);
    if (!is_prime) abort();
    memset(is_prime, true, n + 1);
    is_prime[0] = false;
    is_prime[1] = false;
    for (int64_t i = 2; i <= n; i++) {
        if (is_prime[i]) {
            for (int64_t j = i * i; j <= n; j += i) is_prime[j] = false;
        }
    }
    return is_prime;
}

int main(void) {
    int64_t n = 50000000;
    bool *is_prime = sieve(n);
    int64_t count = 0;
    for (int64_t i = 0; i <= n; i++) count += is_prime[i];
    free(is_prime);
    printf("%ld\n", count);
    return 0;
}
