import { buildIconUrlMap } from "./shared";

const modules = import.meta.glob("../../assets/file-cons-square/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

export default buildIconUrlMap(modules);
