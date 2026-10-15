package main

import "fmt"

func randomArray(n int) []int64 {
	xs := make([]int64, n)
	x := int64(42)
	for i := range xs {
		x = (x*1103515245 + 12345) % 2147483648
		xs[i] = x
	}
	return xs
}

func quicksort(a []int64) {
	if len(a) < 2 {
		return
	}
	hi := len(a) - 1
	pivot := a[hi]
	i := 0
	for j := 0; j < hi; j++ {
		if a[j] < pivot {
			a[i], a[j] = a[j], a[i]
			i++
		}
	}
	a[hi] = a[i]
	a[i] = pivot
	quicksort(a[:i])
	quicksort(a[i+1:])
}

func main() {
	n := 10_000_000
	xs := randomArray(n)
	quicksort(xs)
	sum := int64(0)
	for i, x := range xs {
		sum += x * int64(i%100)
	}
	fmt.Println(sum)
}
