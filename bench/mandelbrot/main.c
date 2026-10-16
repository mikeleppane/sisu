#include <stdint.h>
#include <stdio.h>

int main(void) {
    int64_t n = 4000;
    double size = (double)n;
    int64_t count = 0;
    for (int64_t y = 0; y < n; y++) {
        double ci = 2.0 * (double)y / size - 1.0;
        for (int64_t x = 0; x < n; x++) {
            double cr = 2.0 * (double)x / size - 1.5;
            double zr = 0.0, zi = 0.0, tr = 0.0, ti = 0.0;
            int i = 0;
            while (i < 50 && tr + ti <= 4.0) {
                zi = 2.0 * zr * zi + ci;
                zr = tr - ti + cr;
                tr = zr * zr;
                ti = zi * zi;
                i++;
            }
            if (tr + ti <= 4.0) count++;
        }
    }
    printf("%ld\n", count);
    return 0;
}
