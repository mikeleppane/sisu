# Writes the input of the wc benchmark: 4 000 000 lines of 0 to 12 lowercase words,
# 1 to 10 letters each, separated by one or two spaces or a tab. The
# Park-Miller generator stays exact in awk's doubles, so every awk writes the
# same bytes.
function rand31() { seed = (seed * 16807) % 2147483647; return seed }
BEGIN {
    seed = 42
    for (i = 0; i < 4000000; i++) {
        n = rand31() % 13
        line = ""
        for (w = 0; w < n; w++) {
            if (w > 0) {
                s = rand31() % 8
                line = line (s == 0 ? "\t" : s == 1 ? "  " : " ")
            }
            len = 1 + rand31() % 10
            for (c = 0; c < len; c++) line = line substr("abcdefghijklmnopqrstuvwxyz", 1 + rand31() % 26, 1)
        }
        print line
    }
}
