import React, { useState } from "react";
import type { Source } from "../../types/source";

export const SourceForm: React.FC<{
  onSubmit: (source: Source) => void;
  initialData?: Source;
  isEditing?: boolean;
}> = ({ onSubmit, initialData }) => {
  const [formData, setFormData] = useState<Source>(
    initialData || {
      id: Math.random().toString(36).substring(7),
      name: "",
      kind: "local",
      root: "",
    }
  );

  const handleChange = (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => {
    const { name, value } = e.target;
    setFormData((prev) => ({ ...prev, [name]: value }));
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name || !formData.root) {
      alert("Please fill in all fields");
      return;
    }
    onSubmit(formData);
  };

  return (
    <form id="source-form" onSubmit={handleSubmit} className="space-y-4">
      {/* Name Field */}
      <div className="space-y-2">
        <label htmlFor="name" className="block text-xs font-semibold text-foreground uppercase tracking-wide">
          Source Name
        </label>
        <input
          type="text"
          id="name"
          name="name"
          value={formData.name}
          onChange={handleChange}
          placeholder="e.g., My Documents"
          className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring shadow-sm"
        />
      </div>

      {/* Type Field */}
      <div className="space-y-2">
        <label htmlFor="kind" className="block text-xs font-semibold text-foreground uppercase tracking-wide">
          Storage Type
        </label>
        <select
          name="kind"
          value={formData.kind}
          onChange={handleChange}
          className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring shadow-sm"
        >
          <option value="local">Local Filesystem</option>
          <option value="s3">S3</option>
          <option value="webdav">WebDAV</option>
          <option value="azure_blob">Azure Blob</option>
          <option value="gcs">Google Cloud Storage</option>
        </select>
      </div>

      {/* Path Field */}
      <div className="space-y-2">
        <label htmlFor="root" className="block text-xs font-semibold text-foreground uppercase tracking-wide">
          Path / Root
        </label>
        <input
          type="text"
          id="root"
          name="root"
          value={formData.root}
          onChange={handleChange}
          placeholder={formData.kind === "local" ? "/path/to/folder" : "bucket-name or root path"}
          className="w-full rounded-lg border border-input bg-background px-3 py-2 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-ring shadow-sm"
        />
      </div>
    </form>
  );
};
