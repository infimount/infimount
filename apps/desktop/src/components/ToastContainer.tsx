import React, { useEffect } from "react";
import { useToastStore, Toast } from "../lib/toast";
import clsx from "clsx";

export const ToastContainer: React.FC = () => {
  const toasts = useToastStore((state) => state.toasts);
  const removeToast = useToastStore((state) => state.removeToast);

  return (
    <div className="fixed bottom-4 right-4 z-50 space-y-2 max-w-sm pointer-events-none">
      {toasts.map((toast) => (
        <ToastItem
          key={toast.id}
          toast={toast}
          onClose={() => removeToast(toast.id)}
        />
      ))}
    </div>
  );
};

const ToastItem: React.FC<{
  toast: Toast;
  onClose: () => void;
}> = ({ toast, onClose }) => {
  useEffect(() => {
    if (toast.duration && toast.duration > 0) {
      const timer = setTimeout(onClose, toast.duration);
      return () => clearTimeout(timer);
    }
  }, [toast.duration, onClose]);

  const icons: Record<string, string> = {
    error: "❌",
    success: "✅",
    info: "ℹ️",
    warning: "⚠️",
  };

  const colors = {
    error: "bg-destructive text-destructive-foreground border-destructive",
    success: "bg-green-900 text-green-100 border-green-700",
    info: "bg-blue-900 text-blue-100 border-blue-700",
    warning: "bg-yellow-900 text-yellow-100 border-yellow-700",
  };

  const icon = icons[toast.type] || icons.info;

  return (
    <div
      className={clsx(
        "flex items-center gap-3 rounded-lg border px-4 py-3 shadow-lg animate-fade-in pointer-events-auto",
        colors[toast.type as keyof typeof colors] || colors.info
      )}
    >
      <span className="text-lg flex-shrink-0">{icon}</span>
      <span className="flex-1 text-sm font-medium">{toast.message}</span>
      <button
        className="flex-shrink-0 hover:opacity-70 transition-opacity"
        onClick={onClose}
      >
        ✕
      </button>
    </div>
  );
};
