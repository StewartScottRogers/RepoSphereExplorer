# csvstats-1.0.3-r0.apk

An Alpine Linux package. The extension is the same as an Android
package's, and the two have nothing else in common: this is three gzip
streams laid end to end - the signature, the control entries, then the
files - while an Android package is a zip. They are told apart by
content, which is the only honest way.

Everything a reader wants is in `.PKGINFO` in the control stream. This
one declares two runtime dependencies, provides two commands, and
carries a post-install and a pre-deinstall script, so the pane can say
that installing it runs something.
