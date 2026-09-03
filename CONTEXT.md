# Toolbox Product Context

This glossary defines the product terms used while specifying cross-platform
Toolbox parity and release behavior.

## Release

**Supported target**:
An OS and CPU architecture for which Toolbox publishes a native, tested release artifact. The current targets are macOS arm64 and Windows arm64/x86_64.
_Avoid_: all platforms, universal support

**Release artifact**:
A versioned installer or archive published through GitHub Releases for one supported target.
_Avoid_: build, binary

**Bundled helper**:
A signed native executable or model resource shipped inside a release artifact and selected by the application from its private resources.
_Avoid_: system dependency, PATH tool

**Development override**:
A test-only helper path supplied through an environment variable or PATH; it is not part of the production discovery contract.
_Avoid_: fallback

**Updater manifest**:
A signed, platform-specific description of the release artifacts available to the installed application.
_Avoid_: update URL, latest file

## Vision

**Adapter**:
The stable local process boundary that exposes a vision engine to Toolbox while allowing the engine and model assets to change independently.
_Avoid_: cloud service, model endpoint

**Model baseline**:
The pinned engine version, model file checksum, and fixture results that define an accepted model-dependent behavior.
_Avoid_: latest model, approximate output
