# Backup-Workspace

Archives a workspace: picks the sources, writes a manifest, verifies the
archive it just made, and prunes the old ones.

## Using it

```powershell
./Backup-Workspace.ps1 -Path Z:\repos -Destination D:\backups -Keep 7

# See what it would do, and do nothing:
./Backup-Workspace.ps1 -Path Z:\repos -Destination D:\backups -WhatIf
```

## Notes

- `SupportsShouldProcess`, so `-WhatIf` and `-Confirm` work. A backup
  script that cannot be dry-run is a script nobody dares point at a real
  directory.
- The archive is verified after it is written, before anything old is
  pruned. Deleting yesterday's backup because today's *appeared* to
  succeed is how a backup strategy fails silently.
- `Write-Step` writes to the verbose stream, not the host, so output can be
  captured, redirected or suppressed.

## Developing

```powershell
./build.ps1 -Task All
```

---

**This is a fixture.** It lives in `samples/powershell/` so the application has a
PowerShell project to open, not just a PowerShell file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
