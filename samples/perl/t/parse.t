#!perl
use strict;
use warnings;
use Test2::V0;

use LogDigest;

my $digest = LogDigest->new;

subtest 'a well formed line is parsed' => sub {
    my $line = '127.0.0.1 - - [09/Sep/2026:06:00:00 +0000] "GET /health HTTP/1.1" 200 15';

    my $entry = $digest->parse_line($line);

    ok($entry, 'something came back');
};

subtest 'a line that is not a log line is refused, not guessed at' => sub {
    my $entry = $digest->parse_line('this is not a log line');

    ok(!$entry, 'nothing came back');
};

subtest 'an empty line is not an error' => sub {
    ok(!$digest->parse_line(''), 'an empty line yields nothing');
};

done_testing;
