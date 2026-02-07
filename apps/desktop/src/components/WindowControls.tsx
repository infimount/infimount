import { useState, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "lucide-react";
import { Button } from "@/components/ui/button";

const WINDOW_CONTROL_ICON_PROPS = {
    className: "h-3.5 w-3.5",
    strokeWidth: 1.5,
} as const;

function MaximizeSharpIcon(props: React.SVGProps<SVGSVGElement>) {
    return (
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" {...props}>
            <rect
                x="6.5"
                y="6.5"
                width="11"
                height="11"
                rx="0"
                stroke="currentColor"
                strokeWidth={props.strokeWidth ?? 1.5}
                strokeLinejoin="miter"
            />
        </svg>
    );
}

function RestoreSharpIcon(props: React.SVGProps<SVGSVGElement>) {
    return (
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" {...props}>
            <rect
                x="8.5"
                y="7.5"
                width="9"
                height="9"
                rx="0"
                stroke="currentColor"
                strokeWidth={props.strokeWidth ?? 1.5}
                strokeLinejoin="miter"
            />
            <path
                d="M7.5 16.5H6.5a1 1 0 0 1-1-1V6.5a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v1"
                stroke="currentColor"
                strokeWidth={props.strokeWidth ?? 1.5}
                strokeLinejoin="miter"
                strokeLinecap="square"
            />
        </svg>
    );
}

export function WindowControls() {
    const [isMaximized, setIsMaximized] = useState(false);
    const appWindow = getCurrentWindow();

    useEffect(() => {
        appWindow.isMaximized().then(setIsMaximized).catch(() => { });
        const unlisten = appWindow.listen("tauri://resize", async () => {
            setIsMaximized(await appWindow.isMaximized());
        });
        return () => {
            unlisten.then((f) => f());
        };
    }, []);

    return (
        <div className="flex items-center gap-1 tauri-no-drag relative z-50">
            <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
                onClick={async () => await appWindow.minimize()}
                title="Minimize"
            >
                <Minus {...WINDOW_CONTROL_ICON_PROPS} />
            </Button>
            <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
                onClick={async () => {
                    if (await appWindow.isMaximized()) {
                        await appWindow.unmaximize();
                        setIsMaximized(false);
                    } else {
                        await appWindow.maximize();
                        setIsMaximized(true);
                    }
                }}
                title={isMaximized ? "Restore" : "Maximize"}
            >
                {isMaximized ? (
                    <RestoreSharpIcon {...WINDOW_CONTROL_ICON_PROPS} />
                ) : (
                    <MaximizeSharpIcon {...WINDOW_CONTROL_ICON_PROPS} />
                )}
            </Button>
            <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7 text-muted-foreground hover:bg-neutral-600 hover:text-white"
                onClick={async () => await appWindow.close()}
                title="Close"
            >
                <X {...WINDOW_CONTROL_ICON_PROPS} />
            </Button>
        </div>
    );
}
