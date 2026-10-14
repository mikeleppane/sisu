package main

import (
	"bufio"
	"fmt"
	"os"
	"strings"
)

func main() {
	lines, words := 0, 0
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		lines++
		words += len(strings.Fields(scanner.Text()))
	}
	fmt.Println(lines, words)
}
