# composer.lock

What Composer resolved, in JSON.

`packages` and `packages-dev` are kept apart because only the first is
installed in production, and the `content-hash` is what tells Composer
whether the `composer.json` beside it has changed since.

`platform` is the other half: requirements on PHP itself and on its
extensions, which are not packages and cannot be installed by Composer
- they either exist on the machine or the install fails.
