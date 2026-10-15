fn random_array(n: usize) -> Vec<i64> {
    let mut x: i64 = 42;
    (0..n)
        .map(|_| {
            x = (x * 1103515245 + 12345) % 2147483648;
            x
        })
        .collect()
}

fn quicksort(a: &mut [i64]) {
    if a.len() < 2 {
        return;
    }
    let hi = a.len() - 1;
    let pivot = a[hi];
    let mut i = 0;
    for j in 0..hi {
        if a[j] < pivot {
            a.swap(i, j);
            i += 1;
        }
    }
    a.swap(i, hi);
    let (left, right) = a.split_at_mut(i);
    quicksort(left);
    quicksort(&mut right[1..]);
}

fn main() {
    let n = 10_000_000;
    let mut xs = random_array(n);
    quicksort(&mut xs);
    let sum: i64 = xs.iter().enumerate().map(|(i, &x)| x * (i as i64 % 100)).sum();
    println!("{sum}");
}
