public class Main {
    static final double SOLAR_MASS = 4 * Math.PI * Math.PI;
    static final double DAYS_PER_YEAR = 365.24;

    static final class Body {
        double x, y, z, vx, vy, vz;
        final double mass;

        Body(double x, double y, double z, double vx, double vy, double vz, double mass) {
            this.x = x;
            this.y = y;
            this.z = z;
            this.vx = vx;
            this.vy = vy;
            this.vz = vz;
            this.mass = mass;
        }
    }

    static Body planet(double x, double y, double z, double vx, double vy, double vz, double mass) {
        return new Body(x, y, z, vx * DAYS_PER_YEAR, vy * DAYS_PER_YEAR, vz * DAYS_PER_YEAR,
                mass * SOLAR_MASS);
    }

    static void offsetMomentum(Body[] bodies) {
        double px = 0, py = 0, pz = 0;
        for (Body b : bodies) {
            px += b.vx * b.mass;
            py += b.vy * b.mass;
            pz += b.vz * b.mass;
        }
        bodies[0].vx = -px / SOLAR_MASS;
        bodies[0].vy = -py / SOLAR_MASS;
        bodies[0].vz = -pz / SOLAR_MASS;
    }

    static double energy(Body[] bodies) {
        double e = 0;
        for (int i = 0; i < bodies.length; i++) {
            Body a = bodies[i];
            e += 0.5 * a.mass * (a.vx * a.vx + a.vy * a.vy + a.vz * a.vz);
            for (int j = i + 1; j < bodies.length; j++) {
                Body b = bodies[j];
                double dx = a.x - b.x;
                double dy = a.y - b.y;
                double dz = a.z - b.z;
                e -= a.mass * b.mass / Math.sqrt(dx * dx + dy * dy + dz * dz);
            }
        }
        return e;
    }

    static void advance(Body[] bodies, double dt) {
        for (int i = 0; i < bodies.length; i++) {
            Body a = bodies[i];
            for (int j = i + 1; j < bodies.length; j++) {
                Body b = bodies[j];
                double dx = a.x - b.x;
                double dy = a.y - b.y;
                double dz = a.z - b.z;
                double d2 = dx * dx + dy * dy + dz * dz;
                double mag = dt / (d2 * Math.sqrt(d2));
                a.vx -= dx * b.mass * mag;
                a.vy -= dy * b.mass * mag;
                a.vz -= dz * b.mass * mag;
                b.vx += dx * a.mass * mag;
                b.vy += dy * a.mass * mag;
                b.vz += dz * a.mass * mag;
            }
        }
        for (Body b : bodies) {
            b.x += dt * b.vx;
            b.y += dt * b.vy;
            b.z += dt * b.vz;
        }
    }

    public static void main(String[] args) {
        int steps = 5_000_000;
        Body[] bodies = {
            new Body(0, 0, 0, 0, 0, 0, SOLAR_MASS),
            planet(4.84143144246472090, -1.16032004402742839, -1.03622044471123109e-1,
                    1.66007664274403694e-3, 7.69901118419740425e-3, -6.90460016972063023e-5,
                    9.54791938424326609e-4),
            planet(8.34336671824457987, 4.12479856412430479, -4.03523417114321381e-1,
                    -2.76742510726862411e-3, 4.99852801234917238e-3, 2.30417297573763929e-5,
                    2.85885980666130812e-4),
            planet(1.28943695621391310e1, -1.51111514016986312e1, -2.23307578892655734e-1,
                    2.96460137564761618e-3, 2.37847173959480950e-3, -2.96589568540237556e-5,
                    4.36624404335156298e-5),
            planet(1.53796971148509165e1, -2.59193146099879641e1, 1.79258772950371181e-1,
                    2.68067772490389322e-3, 1.62824170038242295e-3, -9.51592254519715870e-5,
                    5.15138902046611451e-5),
        };
        offsetMomentum(bodies);
        System.out.printf("%.9f%n", energy(bodies));
        for (int i = 0; i < steps; i++) {
            advance(bodies, 0.01);
        }
        System.out.printf("%.9f%n", energy(bodies));
    }
}
