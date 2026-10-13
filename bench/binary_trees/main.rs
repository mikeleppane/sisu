struct Node {
    left: Option<Box<Node>>,
    right: Option<Box<Node>>,
}

impl Node {
    fn check(&self) -> i64 {
        1 + self.left.as_ref().map_or(0, |l| l.check())
            + self.right.as_ref().map_or(0, |r| r.check())
    }
}

fn make(depth: i64) -> Box<Node> {
    if depth == 0 {
        Box::new(Node { left: None, right: None })
    } else {
        Box::new(Node { left: Some(make(depth - 1)), right: Some(make(depth - 1)) })
    }
}

fn main() {
    let min_depth = 4;
    let max_depth = 18;
    println!("{}", make(max_depth + 1).check());
    let long_lived = make(max_depth);
    for depth in (min_depth..=max_depth).step_by(2) {
        let iterations = 1i64 << (max_depth - depth + min_depth);
        let check: i64 = (0..iterations).map(|_| make(depth).check()).sum();
        println!("{iterations}\n{check}");
    }
    println!("{}", long_lived.check());
}
