import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";

import { useSiteCopy } from "@/lib/site-copy";

export default function CommandBlock({ command, label }: { command: string; label: string }) {
  const text = useSiteCopy().install;
  const [status, setStatus] = useState<"ready" | "copied" | "failed">("ready");
  useEffect(() => {
    setStatus("ready");
  }, [command]);
  useEffect(() => {
    if (status === "ready") return;
    const timer = window.setTimeout(() => setStatus("ready"), 2500);
    return () => window.clearTimeout(timer);
  }, [status]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
  }

  return (
    <div className="command-block">
      <div className="command-toolbar">
        <span>{label}</span>
        <button
          className="page-interactive"
          type="button"
          onClick={copy}
          aria-label={`${text.copy}: ${label}`}
        >
          {status === "copied" ? (
            <Check size={16} aria-hidden="true" />
          ) : (
            <Copy size={16} aria-hidden="true" />
          )}
          <span aria-live="polite">
            {status === "copied" ? text.copied : status === "failed" ? text.copyFailed : text.copy}
          </span>
        </button>
      </div>
      <pre>
        <code>{command}</code>
      </pre>
    </div>
  );
}
