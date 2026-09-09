use strict;
use warnings;
use ExtUtils::MakeMaker;

WriteMakefile(
    NAME             => 'LogDigest',
    AUTHOR           => 'Example <engineering@example.com>',
    VERSION_FROM     => 'lib/LogDigest.pm',
    ABSTRACT         => 'Summarises web server logs into counts and rates.',
    LICENSE          => 'mit',
    MIN_PERL_VERSION => '5.036',
    PREREQ_PM        => {
        'Carp'       => 0,
        'List::Util' => '1.55',
        'POSIX'      => 0,
    },
    TEST_REQUIRES => {
        'Test2::V0' => '0.000159',
    },
    META_MERGE => {
        'meta-spec' => { version => 2 },
        resources   => {
            repository => {
                type => 'git',
                url  => 'https://github.com/example/LogDigest.git',
                web  => 'https://github.com/example/LogDigest',
            },
        },
    },
    dist  => { COMPRESS => 'gzip -9f', SUFFIX => 'gz' },
    clean => { FILES => 'LogDigest-*' },
);
