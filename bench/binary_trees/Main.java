public class Main {
    static final class Node {
        final Node left;
        final Node right;

        Node(Node left, Node right) {
            this.left = left;
            this.right = right;
        }

        long check() {
            return 1 + (left != null ? left.check() : 0) + (right != null ? right.check() : 0);
        }
    }

    static Node make(int depth) {
        return depth == 0 ? new Node(null, null) : new Node(make(depth - 1), make(depth - 1));
    }

    public static void main(String[] args) {
        int minDepth = 4;
        int maxDepth = 18;
        System.out.println(make(maxDepth + 1).check());
        Node longLived = make(maxDepth);
        for (int depth = minDepth; depth <= maxDepth; depth += 2) {
            long iterations = 1L << (maxDepth - depth + minDepth);
            long check = 0;
            for (long i = 0; i < iterations; i++) {
                check += make(depth).check();
            }
            System.out.println(iterations);
            System.out.println(check);
        }
        System.out.println(longLived.check());
    }
}
