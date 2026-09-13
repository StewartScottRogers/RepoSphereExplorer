// The shape a readings file has to have, and one that does.

package readings

import (
	"strings"
	"time"
)

// A single reading from one sensor.
#Reading: {
	sensor:  string & strings.MinRunes(3) & strings.MaxRunes(32)
	celsius: >=-90.0 & <=60.0
	taken:   time.Time
	quality: "good" | "suspect" | "missing"
	note?:   string
}

// A column of them, with the sensor they all came from.
#Column: {
	sensor:   string
	unit:     "celsius" | "fahrenheit" | *"celsius"
	readings: [...#Reading]
	count:    >=0 & <=100_000
}

#Retention: {
	days:      int & >=1 & <=3650
	compress:  bool | *true
	archiveTo: string | *"s3://readings/archive"
}

// The concrete instance the collector writes.
column: #Column & {
	sensor: "roof-north"
	count:  3
	readings: [
		{sensor: "roof-north", celsius: 21.4, taken: "2026-09-12T06:00:00Z", quality: "good"},
		{sensor: "roof-north", celsius: 21.9, taken: "2026-09-12T07:00:00Z", quality: "good"},
		{sensor: "roof-north", celsius: 58.2, taken: "2026-09-12T08:00:00Z", quality: "suspect", note: "direct sunlight on the housing"},
	]
}

retention: #Retention & {
	days: 30
}
