import readline from "node:readline";

const input = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

for await (const line of input) {
  const request = JSON.parse(line);
  process.stdout.write(`${JSON.stringify({
    cycle: request.cycle,
    id: request.id,
    protocol: "cua-kernel-control-v1",
  })}\n`);
  process.stderr.write(`kernel-cycle=${request.cycle}\n`);
}
