# flake.nix and default.nix

Two Nix expressions, and the difference between them is the point.

`flake.nix` is a flake: it pins its inputs by name and URL, which is
what makes a build reproducible, and it produces outputs per system - a
package, a development shell and a check. A flake has a lock file
beside it in real use, and everything it depends on is named here
rather than taken from whatever the machine happens to have.

`default.nix` is the older shape: a function taking an attribute set
with defaults, `with pkgs;` in scope, and a `stdenv.mkDerivation` call
describing how to build the package. It names its native build inputs,
its build inputs and its check inputs separately, because Nix keeps
those apart - what is needed to *build* is not what is needed to *run*.
