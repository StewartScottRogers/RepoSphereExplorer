# csvstats-1.0.3.gem

A Ruby gem: a tar archive of three gzip members — `metadata.gz`,
`data.tar.gz` and `checksums.yaml.gz`.

Everything a reader wants is in `metadata.gz`, which is a YAML dump of
the gem specification. It names two runtime dependencies and two
development ones; telling those apart matters, because only the runtime
ones are installed alongside the gem.
