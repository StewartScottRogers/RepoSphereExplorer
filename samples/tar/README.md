# readings.tar

An archive in the PAX format, holding nested directories, a comma
separated file, an executable script, a symbolic link, and a path over a
hundred characters long — which is why it is PAX: a name that will not
fit the old hundred-byte field needs an extended header.

`readings/.secret` is mode 0600 and `readings/report` is 0755. Modes are
the part of an archive nobody looks at until something has been extracted
with the wrong ones.
