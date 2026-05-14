import { buildIconUrlMap } from "./shared";

const modules = import.meta.glob("../../assets/file-icons-vivid/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

export default buildIconUrlMap(modules);
