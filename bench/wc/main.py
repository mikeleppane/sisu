import sys

lines = words = 0
for line in sys.stdin:
    lines += 1
    words += len(line.split())
print(lines, words)
