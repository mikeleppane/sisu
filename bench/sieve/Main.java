import java.util.Arrays;

public class Main {
    static boolean[] sieve(int n) {
        boolean[] isPrime = new boolean[n + 1];
        Arrays.fill(isPrime, true);
        isPrime[0] = false;
        isPrime[1] = false;
        for (int i = 2; i <= n; i++) {
            if (isPrime[i]) {
                for (long j = (long) i * i; j <= n; j += i) {
                    isPrime[(int) j] = false;
                }
            }
        }
        return isPrime;
    }

    public static void main(String[] args) {
        long count = 0;
        for (boolean p : sieve(50_000_000)) {
            if (p) count++;
        }
        System.out.println(count);
    }
}
