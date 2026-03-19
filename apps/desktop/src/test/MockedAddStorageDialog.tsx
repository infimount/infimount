import { AddStorageDialog } from "@/components/AddStorageDialog";
import type {
  StorageDraft,
  StorageKindSchema,
  StorageValidationResult,
} from "@/lib/api";

const schemas: StorageKindSchema[] = [
  {
    id: "local-fs",
    label: "Local Filesystem",
    kind: "local",
    fields: [
      {
        name: "root",
        label: "Root Path",
        input_type: "text",
        required: true,
      },
    ],
  },
  {
    id: "gcs",
    label: "Google Cloud Storage",
    kind: "gcs",
    fields: [
      {
        name: "bucket",
        label: "Bucket",
        input_type: "text",
        required: true,
      },
      {
        name: "service_account_json",
        label: "Service Account JSON",
        input_type: "textarea",
        required: false,
        secret: true,
      },
    ],
  },
];

const validationResult: StorageValidationResult = {
  valid: true,
  details: "Storage validated successfully.",
  capabilities: {
    list: true,
    stat: true,
    read: true,
    write: true,
    delete: true,
    copy: true,
    rename: true,
    presign_read: false,
    create_dir: true,
  },
};

export function MockedAddStorageDialog() {
  return (
    <AddStorageDialog
      open
      onOpenChange={() => undefined}
      loadSchemas={async () => schemas}
      onVerify={async () => validationResult}
      onAdd={async (draft: StorageDraft) => {
        (window as Window & { __PLAYWRIGHT_ADD_STORAGE_RESULT__?: StorageDraft }).__PLAYWRIGHT_ADD_STORAGE_RESULT__ = draft;
      }}
    />
  );
}
