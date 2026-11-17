import { useEffect, useState } from "react";

import CommandBlock from "@/components/command-block";
import {
  detectPlatform,
  INSTALL_PLATFORMS,
  installCommand,
  type InstallPlatform,
} from "@/lib/install";
import { useSiteCopy } from "@/lib/site-copy";

export default function PlatformInstall() {
  const copy = useSiteCopy().install;
  const [platform, setPlatform] = useState<InstallPlatform | null>(null);
  useEffect(() => {
    setPlatform(detectPlatform(navigator));
  }, []);

  return (
    <div className="home-install">
      <div className="platform-picker" role="group" aria-label={copy.platform}>
        {INSTALL_PLATFORMS.map((value) => (
          <button
            className="page-interactive"
            type="button"
            key={value}
            aria-pressed={platform === value}
            onClick={() => setPlatform(value)}
          >
            {copy.platforms[value]}
          </button>
        ))}
      </div>
      {platform ? (
        <CommandBlock
          command={installCommand(platform)}
          label={platform === "windows" ? "PowerShell" : "Bash"}
        />
      ) : (
        <p className="platform-hint">{copy.choose}</p>
      )}
      <p className="platform-hint">{copy.hint}</p>
    </div>
  );
}
