# Security

tridi opens files it didn't write: the Nautilus thumbnailer runs on every
point cloud, mesh or STEP file in a folder you browse. A file that crashes
it, hangs it or makes it read or write outside what it should is a
security bug.

## Reporting

Please don't open a public issue. Use GitHub's private reporting:
**Security → Report a vulnerability** on this repository. Include the file
(or how to make it), the command, and the output of `TRIDI_DEBUG=1 tridi FILE`.

You'll get an answer within a week. Fixes go out in a patch release; the
advisory credits you unless you'd rather it didn't.

## Supported versions

Only the latest release.
