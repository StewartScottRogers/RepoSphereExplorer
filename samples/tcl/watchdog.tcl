#!/usr/bin/env tclsh
#
# A process watchdog: reads a table of checks, runs each on its own
# interval, restarts what fails too often, and writes a status line the
# monitoring system scrapes.

package require Tcl 8.6

namespace eval ::watchdog {
    variable checks [dict create]
    variable failures [dict create]
    variable started [clock seconds]

    namespace export register run status reset
    namespace ensemble create
}

namespace eval ::watchdog::format {
    proc duration {seconds} {
        set minutes [expr {$seconds / 60}]
        set remainder [expr {$seconds % 60}]
        return [format "%dm%02ds" $minutes $remainder]
    }

    proc row {name state count} {
        return [format "%-16s %-8s %3d" $name $state $count]
    }
}

proc ::watchdog::register {name script args} {
    variable checks

    array set options [list -interval 30 -threshold 3 -restart ""]
    array set options $args

    dict set checks $name [dict create \
        script $script \
        interval $options(-interval) \
        threshold $options(-threshold) \
        restart $options(-restart) \
        lastRun 0]

    return $name
}

proc ::watchdog::runOne {name} {
    variable checks
    variable failures

    set check [dict get $checks $name]
    set script [dict get $check script]

    if {[catch {uplevel #0 $script} result options]} {
        dict incr failures $name
        return [list failed $result]
    }

    dict set failures $name 0
    return [list ok $result]
}

proc ::watchdog::run {{now ""}} {
    variable checks
    variable failures

    if {$now eq ""} {
        set now [clock seconds]
    }

    set report {}
    dict for {name check} $checks {
        set due [expr {[dict get $check lastRun] + [dict get $check interval]}]
        if {$now < $due} {
            continue
        }

        dict set checks $name lastRun $now
        lassign [runOne $name] state detail

        set count [expr {[dict exists $failures $name] ? [dict get $failures $name] : 0}]
        if {$state eq "failed" && $count >= [dict get $check threshold]} {
            restart $name
            set state restarted
        }

        lappend report [::watchdog::format::row $name $state $count]
    }

    return $report
}

proc ::watchdog::restart {name} {
    variable checks
    variable failures

    set command [dict get $checks $name restart]
    if {$command eq ""} {
        return 0
    }

    if {[catch {exec {*}$command} problem]} {
        puts stderr "restart of $name failed: $problem"
        return 0
    }

    dict set failures $name 0
    return 1
}

proc ::watchdog::status {} {
    variable checks
    variable failures
    variable started

    set uptime [::watchdog::format::duration [expr {[clock seconds] - $started}]]
    set failing 0
    dict for {name count} $failures {
        if {$count > 0} {
            incr failing
        }
    }

    return "checks=[dict size $checks] failing=$failing uptime=$uptime"
}

proc ::watchdog::reset {{name ""}} {
    variable failures

    if {$name eq ""} {
        set failures [dict create]
    } else {
        dict set failures $name 0
    }
}

# -- a couple of checks, and one run ------------------------------------

::watchdog::register disk {
    if {![file exists /]} {
        error "root filesystem is missing"
    }
    return "root present"
} -interval 0 -threshold 2

::watchdog::register socket {
    error "connection refused"
} -interval 0 -threshold 1 -restart {}

foreach line [::watchdog::run] {
    puts $line
}
puts [::watchdog::status]
