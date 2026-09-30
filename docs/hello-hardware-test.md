# Windows Hello hardware test (H1)

**Status: not run.** CI has no Hello hardware, and nobody working on PR #23 had a Windows PC to run this on. The CNG verify path compiles on `windows-latest` and nothing more has been shown. The external tier stays hard-denied in Phase 1 (`dasdevbot_core::EXTERNAL_TIER_ENABLED = false`) until this test, H1 and H2 have all passed and the Security Director has signed off.

## What it proves

1. `KeyCredentialManager::RequestCreateAsync` creates the non-exportable `dasdevbot-approval` key on this PC. Windows creates it as RSA-2048.
2. `RetrievePublicKeyWithBlobType(X509SubjectPublicKeyInfo)` returns a DER SPKI with OID `1.2.840.113549.1.1.1` (rsaEncryption).
3. `RequestSignAsync` shows Hello, and after a PIN, face or fingerprint check it returns a 256-byte signature.
4. `hello_key::cng::verify` accepts that signature with `CryptImportPublicKeyInfoEx2` and `BCryptVerifySignature(BCRYPT_PAD_PKCS1, SHA-256)`.
5. The same signature is refused for a changed message, a flipped signature byte, and a different SPKI.
6. The pinned fingerprint (`hello_fingerprint`) matches the enrolled key, and enrolling a second, different key is refused.

## Requirements

- Windows 10 22H2 or Windows 11 with Windows Hello set up (PIN at minimum). Record whether it's TPM-backed: `Get-Tpm` → `TpmPresent`, `TpmReady`.
- Rust 1.98.1 (`rustup toolchain install 1.98.1`) and the MSVC build tools.
- An interactive desktop session. Hello doesn't prompt over SSH or in a service session.
- No real credentials. The test uses a scratch key name and a temp database.

## Steps

1. Clone the repo and check out the PR head. Record the commit SHA.
2. Start from a clean key. In PowerShell, run `certutil -csp "Microsoft Passport Key Storage Provider" -key` and note any key containing `dasdevbot-approval`. If one exists from an earlier run, delete it (`certutil -csp "Microsoft Passport Key Storage Provider" -delkey <name>`) or record that the test reused it.
3. Run the ignored hardware test from a normal (non-elevated) terminal:

   ```
   cargo test -p dasdevbotd hello_hardware -- --ignored --nocapture
   ```

4. When Hello prompts, check that the dialog text is the consent prompt `approve post_pr_comment on DasVR/NIL` (decision, action and target). Complete the check.
5. You should get a second Hello prompt from `RequestSignAsync`. Complete that one too.
6. The test prints the SPKI length, the key OID, the signature length and the fingerprint, then asserts the checks listed under "What it proves". Record the full output.
7. Negative check: run the test again and cancel the Hello prompt. The test must report `Windows Hello consent was denied` (or `refused to sign`) and must not report a verified signature.
8. Negative check: with Hello turned off (Settings → Accounts → Sign-in options → remove the PIN, on a test account only), the test must fail with `Windows Hello is not available`.

## Record

| Field | Value |
|---|---|
| Commit SHA | |
| Windows build (`winver`) | |
| TPM present / ready | |
| Hello method (PIN, face, fingerprint) | |
| SPKI length, OID | |
| Signature length | |
| Positive verify | pass / fail |
| Tampered message, byte and key refused | pass / fail |
| Second key refused by the pin | pass / fail |
| Cancel refused | pass / fail |
| Tester, date | |

## Re-enabling the external tier

The external tier is re-enabled only when all of these hold. Re-enabling it is a separate PR.

- H1: this hardware test passes and the table above is filled in.
- H2: the enrolled key is pinned in the audit tip sidecar, enrollment and reset are audit-chain events, and the swap tests pass.
- The Security Director signs off. Then, in the same PR, set `EXTERNAL_TIER_ENABLED = true` and update `Policy::phase1`, `authorize_decision` and their tests.
