import { buildIconUrlMap } from "./shared";

const modules = import.meta.glob("../../assets/file-icons-modern/*.svg", {
  eager: true,
  import: "default",
}) as Record<string, string>;

export default buildIconUrlMap(modules);
