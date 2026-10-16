fn fannkuch(n: usize) {
    let mut perm1: Vec<usize> = (0..n).collect();
    let mut perm = vec![0; n];
    let mut count = vec![0; n];
    let (mut max_flips, mut checksum, mut perm_count, mut r) = (0i64, 0i64, 0i64, n);
    loop {
        while r != 1 {
            count[r - 1] = r;
            r -= 1;
        }
        perm.copy_from_slice(&perm1);
        let mut flips = 0;
        let mut k = perm[0];
        while k != 0 {
            perm[..=k].reverse();
            flips += 1;
            k = perm[0];
        }
        max_flips = max_flips.max(flips);
        checksum += if perm_count % 2 == 0 { flips } else { -flips };
        loop {
            if r == n {
                println!("{checksum}\nPfannkuchen({n}) = {max_flips}");
                return;
            }
            let first = perm1[0];
            for i in 0..r {
                perm1[i] = perm1[i + 1];
            }
            perm1[r] = first;
            count[r] -= 1;
            if count[r] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
}

fn main() {
    fannkuch(10);
}
