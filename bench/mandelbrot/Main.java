public class Main {
    public static void main(String[] args) {
        int n = 4000;
        double size = n;
        long count = 0;
        for (int y = 0; y < n; y++) {
            double ci = 2.0 * y / size - 1.0;
            for (int x = 0; x < n; x++) {
                double cr = 2.0 * x / size - 1.5;
                double zr = 0, zi = 0, tr = 0, ti = 0;
                int i = 0;
                while (i < 50 && tr + ti <= 4.0) {
                    zi = 2.0 * zr * zi + ci;
                    zr = tr - ti + cr;
                    tr = zr * zr;
                    ti = zi * zi;
                    i++;
                }
                if (tr + ti <= 4.0) {
                    count++;
                }
            }
        }
        System.out.println(count);
    }
}
