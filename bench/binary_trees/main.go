package main

import "fmt"

type Node struct {
	left, right *Node
}

func (n *Node) check() int64 {
	c := int64(1)
	if n.left != nil {
		c += n.left.check()
	}
	if n.right != nil {
		c += n.right.check()
	}
	return c
}

func build(depth int64) *Node {
	if depth == 0 {
		return &Node{}
	}
	return &Node{build(depth - 1), build(depth - 1)}
}

func main() {
	const minDepth, maxDepth = 4, 18
	fmt.Println(build(maxDepth + 1).check())
	longLived := build(maxDepth)
	for depth := int64(minDepth); depth <= maxDepth; depth += 2 {
		iterations := int64(1) << (maxDepth - depth + minDepth)
		check := int64(0)
		for i := int64(0); i < iterations; i++ {
			check += build(depth).check()
		}
		fmt.Println(iterations)
		fmt.Println(check)
	}
	fmt.Println(longLived.check())
}
