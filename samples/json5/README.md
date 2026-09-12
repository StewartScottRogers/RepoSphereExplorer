# config.json5

The Repos Explorer service, configured the way a person writes it rather
than the way a machine emits it.

Every liberty the file takes is one a strict JSON reader refuses:
unquoted keys, single-quoted strings, trailing commas, both comment
forms, hexadecimal numbers, a leading decimal point and a string
continued across lines. That is the point of the fixture — the pane
names each one and says what a strict reader would do with it.
