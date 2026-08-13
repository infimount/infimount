export function canonicalVersionFromTag(tag) {
  const version = String(tag ?? "").replace(/^v/, "");
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)) {
    throw new Error(`unsupported release tag: ${tag}`);
  }
  return version;
}

export function windowsMsiVersion(version) {
  const match = canonicalVersionFromTag(version).match(/^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$/);
  const [, major, minor, patch, prerelease] = match;
  for (const [name, value, maximum] of [
    ["major", major, 255],
    ["minor", minor, 255],
    ["patch", patch, 65535],
  ]) {
    if (Number(value) > maximum) throw new Error(`MSI ${name} version exceeds ${maximum}`);
  }
  if (!prerelease) return `${major}.${minor}.${patch}`;

  const numericIdentifier = prerelease
    .split(".")
    .findLast((identifier) => /^\d+$/.test(identifier));
  if (numericIdentifier === undefined) {
    throw new Error("MSI prerelease tags must contain a numeric identifier (for example rc.1)");
  }
  const build = Number(numericIdentifier);
  if (build > 65535) throw new Error("MSI prerelease identifier exceeds 65535");
  return `${major}.${minor}.${patch}.${build}`;
}
