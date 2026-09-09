# Applied to every build in this directory, which keeps the flags in one
# place rather than in whichever command somebody last remembered.

switch("hint", "Processing:off")
switch("warning", "ObservableStores:off")

when defined(release):
  switch("checks", "off")
