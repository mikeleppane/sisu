public class Main {
    static long[] randomArray(int n) {
        long[] xs = new long[n];
        long x = 42;
        for (int i = 0; i < n; i++) {
            x = (x * 1103515245 + 12345) % 2147483648L;
            xs[i] = x;
        }
        return xs;
    }

    static void quicksort(long[] a, int lo, int hi) {
        if (lo >= hi) return;
        long pivot = a[hi];
        int i = lo;
        for (int j = lo; j < hi; j++) {
            if (a[j] < pivot) {
                long t = a[i];
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

    public static void main(String[] args) {
        int n = 10_000_000;
        long[] xs = randomArray(n);
        quicksort(xs, 0, n - 1);
        long sum = 0;
        for (int i = 0; i < n; i++) sum += xs[i] * (i % 100);
        System.out.println(sum);
    }
}
