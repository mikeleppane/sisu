use std::f64::consts::PI;

const SOLAR_MASS: f64 = 4.0 * PI * PI;
const DAYS_PER_YEAR: f64 = 365.24;

struct Body {
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    mass: f64,
}

fn planet(x: f64, y: f64, z: f64, vx: f64, vy: f64, vz: f64, mass: f64) -> Body {
    Body {
        x,
        y,
        z,
        vx: vx * DAYS_PER_YEAR,
        vy: vy * DAYS_PER_YEAR,
        vz: vz * DAYS_PER_YEAR,
        mass: mass * SOLAR_MASS,
    }
}

fn offset_momentum(bodies: &mut [Body]) {
    let (mut px, mut py, mut pz) = (0.0, 0.0, 0.0);
    for b in bodies.iter() {
        px += b.vx * b.mass;
        py += b.vy * b.mass;
        pz += b.vz * b.mass;
    }
    bodies[0].vx = -px / SOLAR_MASS;
    bodies[0].vy = -py / SOLAR_MASS;
    bodies[0].vz = -pz / SOLAR_MASS;
}

fn energy(bodies: &[Body]) -> f64 {
    let mut e = 0.0;
    for i in 0..bodies.len() {
        let a = &bodies[i];
        e += 0.5 * a.mass * (a.vx * a.vx + a.vy * a.vy + a.vz * a.vz);
        for b in &bodies[i + 1..] {
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let dz = a.z - b.z;
            e -= a.mass * b.mass / (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }
    e
}

fn advance(bodies: &mut [Body], dt: f64) {
    for i in 0..bodies.len() {
        let (head, tail) = bodies.split_at_mut(i + 1);
        let a = &mut head[i];
        for b in tail.iter_mut() {
            let dx = a.x - b.x;
            let dy = a.y - b.y;
            let dz = a.z - b.z;
            let d2 = dx * dx + dy * dy + dz * dz;
            let mag = dt / (d2 * d2.sqrt());
            a.vx -= dx * b.mass * mag;
            a.vy -= dy * b.mass * mag;
            a.vz -= dz * b.mass * mag;
            b.vx += dx * a.mass * mag;
            b.vy += dy * a.mass * mag;
            b.vz += dz * a.mass * mag;
        }
    }
    for b in bodies.iter_mut() {
        b.x += dt * b.vx;
        b.y += dt * b.vy;
        b.z += dt * b.vz;
    }
}

fn main() {
    let steps = 5_000_000;
    let mut bodies = [
        Body { x: 0.0, y: 0.0, z: 0.0, vx: 0.0, vy: 0.0, vz: 0.0, mass: SOLAR_MASS },
        planet(
            4.84143144246472090, -1.16032004402742839, -1.03622044471123109e-1,
            1.66007664274403694e-3, 7.69901118419740425e-3, -6.90460016972063023e-5,
            9.54791938424326609e-4,
        ),
        planet(
            8.34336671824457987, 4.12479856412430479, -4.03523417114321381e-1,
            -2.76742510726862411e-3, 4.99852801234917238e-3, 2.30417297573763929e-5,
            2.85885980666130812e-4,
        ),
        planet(
            1.28943695621391310e1, -1.51111514016986312e1, -2.23307578892655734e-1,
            2.96460137564761618e-3, 2.37847173959480950e-3, -2.96589568540237556e-5,
            4.36624404335156298e-5,
        ),
        planet(
            1.53796971148509165e1, -2.59193146099879641e1, 1.79258772950371181e-1,
            2.68067772490389322e-3, 1.62824170038242295e-3, -9.51592254519715870e-5,
            5.15138902046611451e-5,
        ),
    ];
    offset_momentum(&mut bodies);
    println!("{:.9}", energy(&bodies));
    for _ in 0..steps {
        advance(&mut bodies, 0.01);
    }
    println!("{:.9}", energy(&bodies));
}
