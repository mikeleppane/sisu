public class Main {
    static void fannkuch(int n) {
        int[] perm1 = new int[n];
        for (int i = 0; i < n; i++) {
            perm1[i] = i;
        }
        int[] perm = new int[n];
        int[] count = new int[n];
        long maxFlips = 0, checksum = 0, permCount = 0;
        int r = n;
        while (true) {
            while (r != 1) {
                count[r - 1] = r;
                r--;
            }
            System.arraycopy(perm1, 0, perm, 0, n);
            long flips = 0;
            int k = perm[0];
            while (k != 0) {
                for (int i = 0, j = k; i < j; i++, j--) {
                    int t = perm[i];
                    perm[i] = perm[j];
                    perm[j] = t;
                }
                flips++;
                k = perm[0];
            }
            maxFlips = Math.max(maxFlips, flips);
            checksum += permCount % 2 == 0 ? flips : -flips;
            while (true) {
                if (r == n) {
                    System.out.println(checksum);
                    System.out.println("Pfannkuchen(" + n + ") = " + maxFlips);
                    return;
                }
                int first = perm1[0];
                for (int i = 0; i < r; i++) {
                    perm1[i] = perm1[i + 1];
                }
                perm1[r] = first;
                count[r]--;
                if (count[r] > 0) {
                    break;
                }
                r++;
            }
            permCount++;
        }
    }

    public static void main(String[] args) {
        fannkuch(10);
    }
}
