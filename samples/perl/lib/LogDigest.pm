#!/usr/bin/perl
# A log digest: parses common-format access logs, groups them by status
# class and path, and prints the slowest handful. Written as a module with
# a small script entry point at the bottom.

package LogDigest;

use strict;
use warnings;
use feature qw(say);

use Carp qw(croak);
use List::Util qw(sum0 max);
use POSIX qw(strftime);

our $VERSION = '1.04';

my $LINE = qr{
    ^(?<host>\S+)              \s+
    \S+                        \s+
    (?<user>\S+)               \s+
    \[(?<stamp>[^\]]+)\]       \s+
    "(?<method>[A-Z]+)\s(?<path>\S+)[^"]*"  \s+
    (?<status>\d{3})           \s+
    (?<bytes>\d+|-)            \s+
    (?<millis>\d+)
}x;

sub new {
    my ( $class, %args ) = @_;
    my $self = {
        entries    => [],
        slow_after => $args{slow_after} // 500,
        parsed     => 0,
        skipped    => 0,
    };
    return bless $self, $class;
}

sub parse_line {
    my ( $self, $line ) = @_;
    chomp $line;

    if ( $line !~ $LINE ) {
        $self->{skipped}++;
        return;
    }

    my %entry = (
        host   => $+{host},
        user   => $+{user},
        method => $+{method},
        path   => $+{path},
        status => $+{status} + 0,
        bytes  => $+{bytes} eq '-' ? 0 : $+{bytes} + 0,
        millis => $+{millis} + 0,
    );

    push @{ $self->{entries} }, \%entry;
    $self->{parsed}++;
    return \%entry;
}

sub parse_handle {
    my ( $self, $handle ) = @_;
    croak 'parse_handle needs a filehandle' unless ref $handle;

    while ( my $line = <$handle> ) {
        $self->parse_line($line);
    }
    return $self->{parsed};
}

sub entries { return @{ $_[0]->{entries} } }

sub count { return scalar @{ $_[0]->{entries} } }

sub by_status_class {
    my ($self) = @_;
    my %classes;
    for my $entry ( $self->entries ) {
        my $class = int( $entry->{status} / 100 ) . 'xx';
        $classes{$class}++;
    }
    return \%classes;
}

sub bytes_by_path {
    my ($self) = @_;
    my %totals;
    for my $entry ( $self->entries ) {
        $totals{ $entry->{path} } += $entry->{bytes};
    }
    return \%totals;
}

sub slowest {
    my ( $self, $wanted ) = @_;
    $wanted //= 5;
    my @sorted = sort { $b->{millis} <=> $a->{millis} } $self->entries;
    return @sorted[ 0 .. ( $wanted - 1 < $#sorted ? $wanted - 1 : $#sorted ) ];
}

sub average_millis {
    my ($self) = @_;
    return 0 unless $self->count;
    return sum0( map { $_->{millis} } $self->entries ) / $self->count;
}

sub report {
    my ($self) = @_;
    my @lines;

    push @lines, sprintf 'parsed %d, skipped %d', $self->{parsed}, $self->{skipped};

    my $classes = $self->by_status_class;
    for my $class ( sort keys %{$classes} ) {
        push @lines, sprintf '  %s %5d', $class, $classes->{$class};
    }

    push @lines, sprintf 'average %.1fms, worst %dms', $self->average_millis,
        max( map { $_->{millis} } $self->entries ) // 0;

    for my $entry ( $self->slowest(3) ) {
        push @lines, sprintf '  %5dms %s %s', $entry->{millis}, $entry->{method}, $entry->{path};
    }

    return join "\n", @lines;
}

package main;

use strict;
use warnings;

my $digest = LogDigest->new( slow_after => 250 );

while ( my $line = <DATA> ) {
    $digest->parse_line($line);
}

say $digest->report;

__DATA__
10.0.0.4 - alice [08/Sep/2026:09:12:41 +0000] "GET /index.html HTTP/1.1" 200 5312 41
10.0.0.9 - - [08/Sep/2026:09:12:44 +0000] "GET /assets/app.js HTTP/1.1" 200 91233 128
10.0.0.4 - alice [08/Sep/2026:09:13:02 +0000] "POST /api/orders HTTP/1.1" 201 412 812
10.0.0.7 - bob [08/Sep/2026:09:13:19 +0000] "GET /api/orders/9 HTTP/1.1" 404 122 17
10.0.0.7 - bob [08/Sep/2026:09:13:51 +0000] "GET /api/report HTTP/1.1" 500 88 2411
this line is not a log entry at all
