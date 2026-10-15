# Test data of the restic download

- `restic-0.19.1/` holds the real `SHA256SUMS` and `SHA256SUMS.asc` of the restic 0.19.1 release. The signature test uses them with the pinned release key from `packaging/restic-release-key.asc`.
- `signed-release/` holds a fake release that a throw-away test key signed (RSA 3072, SHA-512). The private key does not exist any more. The archives hold a shell script that prints a restic version. `SHA256SUMS` lists the same archive under several versions, so a test can copy the archive to a name with another version. One entry has an all-zero hash. `other-key.asc` is a second test key, for the wrong key test.
