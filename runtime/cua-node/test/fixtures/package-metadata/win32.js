export function pathListSeparator(platform = process.platform) {
  return platform === "win32" ? ";" : ":";
}
