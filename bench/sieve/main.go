package main

import "fmt"

func sieve(n int) []bool {
	isPrime := make([]bool, n+1)
	for i := range isPrime {
		isPrime[i] = true
	}
	isPrime[0] = false
	isPrime[1] = false
	for i := 2; i <= n; i++ {
		if isPrime[i] {
			for j := i * i; j <= n; j += i {
				isPrime[j] = false
			}
		}
	}
	return isPrime
}

func main() {
	count := 0
	for _, p := range sieve(50_000_000) {
		if p {
			count++
		}
	}
	fmt.Println(count)
}
