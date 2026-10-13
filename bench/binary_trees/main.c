#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

struct node {
    struct node *left;
    struct node *right;
};

static struct node *make(int64_t depth) {
    struct node *n = malloc(sizeof *n);
    if (!n) abort();
    if (depth == 0) {
        n->left = NULL;
        n->right = NULL;
    } else {
        n->left = make(depth - 1);
        n->right = make(depth - 1);
    }
    return n;
}

static int64_t check(const struct node *n) {
    return 1 + (n->left ? check(n->left) : 0) + (n->right ? check(n->right) : 0);
}

static void destroy(struct node *n) {
    if (n->left) destroy(n->left);
    if (n->right) destroy(n->right);
    free(n);
}

int main(void) {
    int64_t min_depth = 4;
    int64_t max_depth = 18;
    struct node *stretch = make(max_depth + 1);
    printf("%ld\n", check(stretch));
    destroy(stretch);
    struct node *long_lived = make(max_depth);
    for (int64_t depth = min_depth; depth <= max_depth; depth += 2) {
        int64_t iterations = (int64_t)1 << (max_depth - depth + min_depth);
        int64_t sum = 0;
        for (int64_t i = 0; i < iterations; i++) {
            struct node *t = make(depth);
            sum += check(t);
            destroy(t);
        }
        printf("%ld\n%ld\n", iterations, sum);
    }
    printf("%ld\n", check(long_lived));
    destroy(long_lived);
    return 0;
}
