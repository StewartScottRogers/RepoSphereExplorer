# csvstats-1.0.3.apk

An Android package: a zip holding a **binary** `AndroidManifest.xml`,
two `classes.dex` files, a compiled resource table and native libraries
for two architectures.

The manifest is compiled, not text — which is why reading a package
means decoding Android's own binary XML rather than parsing angle
brackets. It asks for three permissions, one of them location.
