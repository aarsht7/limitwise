# LimitWise

LimitWise schedules Codex work while respecting rolling five-hour and weekly usage limits.

> **Compatibility warning:** LimitWise has only been tested on Linux x86-64. macOS, including Apple Silicon, and other architectures are untested and require confirmation during installation.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/aarsht7/limitwise/main/install.sh | sh
```

The installer verifies the release checksum, installs the plugin, then asks separately before installing the background service. Open a new Codex conversation when it finishes.

Technical users can install the GitHub marketplace directly:

```sh
codex plugin marketplace add aarsht7/limitwise
codex plugin add limitwise@limitwise
```

Direct marketplace installation does not install a prebuilt binary or background service. Use the installer above for the complete setup.

Plugin source and documentation live in [`plugins/limitwise`](plugins/limitwise/README.md).
