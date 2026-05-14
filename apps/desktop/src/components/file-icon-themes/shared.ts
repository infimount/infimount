export const buildIconUrlMap = (modules: Record<string, string>) => {
  const map = new Map<string, string>();
  Object.entries(modules).forEach(([path, url]) => {
    const name = path.split("/").pop()?.replace(".svg", "").toLowerCase();
    if (name) {
      map.set(name, url);
    }
  });
  return map;
};
