# Who may read and write the readings, and who may not.

package readings.authz

import future.keywords.contains
import future.keywords.if
import future.keywords.in

import data.readings.roles
import data.readings.sensors

default allow := false
default reason := "no rule matched"
default max_rows := 1000

# Anybody may read a sensor their team owns.
allow if {
	input.action == "read"
	some team in roles[input.user].teams
	sensors[input.resource].team == team
}

# An operator may write, but only during their shift and only from the
# collector's own network.
allow if {
	input.action == "write"
	"operator" in roles[input.user].titles
	within_shift(input.time)
	net.cidr_contains("10.0.0.0/24", input.source_ip)
}

# An administrator may do anything, which is why there are few of them.
allow if {
	"admin" in roles[input.user].titles
}

# Partial: every reason a request was refused, so the caller is told all
# of them rather than the first.
deny contains message if {
	input.action == "write"
	not within_shift(input.time)
	message := sprintf("outside %v's shift", [input.user])
}

deny contains message if {
	input.action == "write"
	count(input.rows) > max_rows
	message := sprintf("%d rows is more than %d", [count(input.rows), max_rows])
}

reason := "allowed" if allow

# The shift runs from six in the morning to six at night.
within_shift(moment) if {
	hour := time.clock(moment)[0]
	hour >= 6
	hour < 18
}

readable_sensors[name] {
	sensors[name].team in roles[input.user].teams
}
