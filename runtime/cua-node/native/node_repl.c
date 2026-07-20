/* Reproducible freestanding Linux x86-64 launcher for the private Node host. */
typedef unsigned long usize;

static long syscall3(long number, long first, long second, long third) {
  long result;
  __asm__ volatile("syscall"
                   : "=a"(result)
                   : "a"(number), "D"(first), "S"(second), "d"(third)
                   : "rcx", "r11", "memory");
  return result;
}

static usize length(const char *value) {
  usize size = 0;
  while (value[size] != '\0') size++;
  return size;
}

static int equal(const char *left, const char *right) {
  usize index = 0;
  while (left[index] != '\0' && right[index] != '\0') {
    if (left[index] != right[index]) return 0;
    index++;
  }
  return left[index] == right[index];
}

static void append(char *destination, usize *position, const char *source) {
  for (usize index = 0; source[index] != '\0'; index++) {
    destination[(*position)++] = source[index];
  }
}

static void launch(long argc, char **argv, char **envp) {
  if (argc == 2 && equal(argv[1], "--version")) {
    const char *identity = "node_repl/0.1.0\n";
    syscall3(1, 1, (long)identity, (long)length(identity));
    syscall3(60, 0, 0, 0);
  }
  char executable[4096];
  long read = syscall3(89, (long)"/proc/self/exe", (long)executable, 4095);
  if (read <= 0 || read >= 4095) syscall3(1, 2, (long)"node_repl: cannot resolve launcher\n", 35);
  if (read <= 0 || read >= 4095) syscall3(60, 126, 0, 0);
  executable[read] = '\0';

  usize directory = length(executable);
  while (directory > 0 && executable[directory - 1] != '/') directory--;
  if (directory == 0) syscall3(60, 126, 0, 0);

  char node[4096];
  char host[4096];
  usize nodeSize = 0;
  usize hostSize = 0;
  for (usize index = 0; index < directory; index++) {
    node[nodeSize++] = executable[index];
    host[hostSize++] = executable[index];
  }
  append(node, &nodeSize, "node");
  append(host, &hostSize, "../lib/node_repl/cli.js");
  node[nodeSize] = '\0';
  host[hostSize] = '\0';

  char *arguments[4096];
  if (argc + 2 >= 4096) syscall3(60, 126, 0, 0);
  arguments[0] = node;
  arguments[1] = host;
  for (long index = 1; index < argc; index++) arguments[index + 1] = argv[index];
  arguments[argc + 1] = (char *)0;
  syscall3(59, (long)node, (long)arguments, (long)envp);
  syscall3(1, 2, (long)"node_repl: bundled Node exec failed\n", 36);
  syscall3(60, 127, 0, 0);
}

__attribute__((noreturn, used)) void start_from_stack(long *stack) {
  long argc = stack[0];
  char **argv = (char **)&stack[1];
  char **envp = &argv[argc + 1];
  launch(argc, argv, envp);
  __builtin_unreachable();
}

__attribute__((naked, noreturn)) void _start(void) {
  __asm__ volatile("mov %rsp, %rdi\n"
                   "andq $-16, %rsp\n"
                   "call start_from_stack\n");
}
