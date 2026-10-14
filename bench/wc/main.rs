use std::io::BufRead;

fn main() {
    let (mut lines, mut words) = (0, 0);
    for line in std::io::stdin().lock().lines() {
        lines += 1;
        words += line.unwrap().split_whitespace().count();
    }
    println!("{lines} {words}");
}
