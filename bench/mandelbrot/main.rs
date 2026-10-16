fn main() {
    let n: i64 = 4000;
    let size = n as f64;
    let mut count = 0;
    for y in 0..n {
        let ci = 2.0 * y as f64 / size - 1.0;
        for x in 0..n {
            let cr = 2.0 * x as f64 / size - 1.5;
            let (mut zr, mut zi, mut tr, mut ti) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut i = 0;
            while i < 50 && tr + ti <= 4.0 {
                zi = 2.0 * zr * zi + ci;
                zr = tr - ti + cr;
                tr = zr * zr;
                ti = zi * zi;
                i += 1;
            }
            if tr + ti <= 4.0 {
                count += 1;
            }
        }
    }
    println!("{count}");
}
