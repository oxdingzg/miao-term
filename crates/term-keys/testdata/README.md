# Test fixtures for `mtty-keys`

These `.ppk` files were generated for the tests only, with `puttygen` 0.81,
from throwaway keys. They are not used to reach anything and must never be.

- `<key>_v2.ppk` / `<key>_v3.ppk`: unencrypted
- `<key>_v2_enc.ppk`: passphrase `mtty-test-v2`
- `<key>_v3_enc.ppk`: passphrase `mtty-test-v3`
- `<key>_plain.pub`: the OpenSSH public key the fixture must import to

Keys: `ed25519`, `rsa` (2048), `ecdsa` (nistp256).
