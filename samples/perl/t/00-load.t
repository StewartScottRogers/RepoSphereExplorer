#!perl
use strict;
use warnings;
use Test2::V0;

use_ok('LogDigest');

ok(defined $LogDigest::VERSION, 'the module states a version');
like($LogDigest::VERSION, qr/^\d+\.\d+$/, 'and it looks like one');

done_testing;
