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

interface AddStorageDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd?: (config: { name: string; type: StorageType; config: Record<string, string> }) => void;
  onUpdate?: (id: string, config: { name: string; type: StorageType; config: Record<string, string> }) => void;
  initialStorage?: StorageConfig;
}

const storageFields: Record<StorageType, { label: string; key: string; type?: string }[]> = {
  'aws-s3': [
    { label: 'Bucket Name', key: 'bucketName' },
    { label: 'Region', key: 'region' },
    { label: 'Access Key ID', key: 'accessKeyId' },
    { label: 'Secret Access Key', key: 'secretAccessKey', type: 'password' },
  ],
  'azure-blob': [
    { label: 'Account Name', key: 'accountName' },
    { label: 'Container Name', key: 'containerName' },
    { label: 'Account Key', key: 'accountKey', type: 'password' },
  ],
  'webdav': [
    { label: 'Server URL', key: 'serverUrl' },
    { label: 'Username', key: 'username' },
    { label: 'Password', key: 'password', type: 'password' },
    { label: 'Root Path', key: 'rootPath' },
  ],
  'local-fs': [
    { label: 'Root Folder Path', key: 'rootPath' },
  ],
};

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

  const fields = storageFields[type];

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
                <SelectItem value="aws-s3">AWS S3</SelectItem>
                <SelectItem value="azure-blob">Azure Blob Storage</SelectItem>
                <SelectItem value="webdav">WebDAV</SelectItem>
                <SelectItem value="local-fs">Local File System</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {fields.map((field) => (
            <div key={field.key} className="space-y-2">
              <Label htmlFor={field.key}>{field.label}</Label>
              <Input
                id={field.key}
                type={field.type || 'text'}
                value={config[field.key] || ''}
                onChange={(e) =>
                  setConfig({ ...config, [field.key]: e.target.value })
                }
                placeholder={field.label}
                required
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
