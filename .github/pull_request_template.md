## What this changes

<!-- One or two lines. -->

## Checks

- [ ] `.github/scripts/check.sh` passes
- [ ] Ran it on a real Omarchy machine, not just CI

## If it touches the firewall or `setup`

- [ ] `sudo ./tormarchy connect --dry-run strict | sudo nft -c -f -` is accepted
- [ ] `tormarchy doctor` still reports no leaks while connected
- [ ] `tormarchy uninstall` still puts everything back

Delete that section if it doesn't apply.
