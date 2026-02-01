import { useState, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Maximize2 } from "lucide-react";
import { Button } from "@/components/ui/button";

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
                className="h-8 w-8 text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
                onClick={async () => await appWindow.minimize()}
                title="Minimize"
            >
                <Minus className="h-4 w-4" />
            </Button>
            <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
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
                    <Maximize2 className="h-3 w-3 rotate-180" />
                ) : (
                    <Square className="h-3.5 w-3.5" />
                )}
            </Button>
            <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 text-muted-foreground hover:bg-neutral-600 hover:text-white"
                onClick={async () => await appWindow.close()}
                title="Close"
            >
                <X className="h-4 w-4" />
            </Button>
        </div>
    );
}
