import { useEffect, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { StorageType, type StorageConfig } from "@/types/storage";
import { listStorageSchemas, type StorageKindSchema } from "@/lib/api";

interface AddStorageDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd?: (config: { name: string; type: StorageType; config: Record<string, string> }) => void;
  onUpdate?: (id: string, config: { name: string; type: StorageType; config: Record<string, string> }) => void;
  initialStorage?: StorageConfig;
}

export function AddStorageDialog({
  open,
  onOpenChange,
  onAdd,
  onUpdate,
  initialStorage,
}: AddStorageDialogProps) {
  const isEditing = Boolean(initialStorage);
  const [name, setName] = useState(initialStorage?.name ?? "");
  const [type, setType] = useState<StorageType>(initialStorage?.type ?? "aws-s3");
  const [config, setConfig] = useState<Record<string, string>>(initialStorage?.config ?? {});
  const [schemas, setSchemas] = useState<StorageKindSchema[]>([]);

  useEffect(() => {
    if (initialStorage && open) {
      setName(initialStorage.name);
      setType(initialStorage.type);
      setConfig(initialStorage.config ?? {});
    }
    if (!open && !initialStorage) {
      setName("");
      setType("aws-s3");
      setConfig({});
    }
  }, [initialStorage, open]);

  useEffect(() => {
    let mounted = true;
    listStorageSchemas()
      .then((items) => {
        if (!mounted) return;
        setSchemas(items);
        if (!isEditing && !initialStorage && items.length > 0) {
          setType(items[0].id as StorageType);
        }
      })
      // eslint-disable-next-line no-console
      .catch((err) => console.error("Failed to load storage schemas", err));
    return () => {
      mounted = false;
    };
    // We only want this once on mount/edit-load.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (isEditing && initialStorage && onUpdate) {
      onUpdate(initialStorage.id, { name, type, config });
    } else if (onAdd) {
      onAdd({ name, type, config });
    }
    if (!isEditing) {
      setName("");
      setType("aws-s3");
      setConfig({});
    }
    onOpenChange(false);
  };

  const currentSchema = schemas.find((s) => s.id === type);
  const fields = currentSchema?.fields ?? [];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px] rounded-2xl border border-border bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] shadow-2xl">
        <DialogHeader>
          <DialogTitle>{isEditing ? "Edit Storage" : "Add New Storage"}</DialogTitle>
          <DialogDescription>
            {isEditing
              ? "Update the storage configuration and save your changes."
              : "Configure a new storage connection. Choose the storage type and fill in the required credentials."}
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="name">Storage Name</Label>
            <Input
              id="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Storage"
              required
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="type">Storage Type</Label>
            <Select value={type} onValueChange={(v) => setType(v as StorageType)}>
              <SelectTrigger
                id="type"
                className="bg-[hsl(var(--card))] text-[hsl(var(--card-foreground))] border border-border"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent className="bg-[hsl(var(--popover))] text-[hsl(var(--popover-foreground))] border border-border shadow-md">
                {schemas.map((schema) => (
                  <SelectItem key={schema.id} value={schema.id}>
                    {schema.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {fields.map((field) => (
            <div key={field.name} className="space-y-2">
              <Label htmlFor={field.name}>{field.label}</Label>
              <Input
                id={field.name}
                type={field.input_type === "password" || field.secret ? "password" : "text"}
                value={config[field.name] || ""}
                onChange={(e) =>
                  setConfig({ ...config, [field.name]: e.target.value })
                }
                placeholder={field.label}
                required={field.required}
              />
            </div>
          ))}

          <div className="flex justify-end gap-3 pt-4">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit">{isEditing ? "Save Changes" : "Add Storage"}</Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
