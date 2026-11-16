# journal

- 2026-11-16: discovery changed from ARP requests (spec) to mDNS browsing + TCP port sweep + system reverse lookup, because ARP needs raw socket privileges. specifications.md still mentions ARP, pending architect update.
- 2026-11-16: chose tokio + russh over threads + ssh2 (shared multiplexed sftp session, uniform cancellation, no libssh2/openssl C build). to check: rust toolchain version in the Sailfish SDK vs tokio/russh MSRV; fallback is threads + ssh2.
