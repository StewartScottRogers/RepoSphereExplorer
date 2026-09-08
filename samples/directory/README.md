# A directory, previewed as a directory

`samples/directory/` is the fixture for the `directory` plugin, which is
the one plugin with no file of its own to sniff: the directory *is* what
it recognises, and its preview reports what the folder holds rather than
what any one file contains.

So this folder needs contents that make that preview say something. It
holds a handful of files of different sizes and types, which is enough
for the plugin to report an entry count and a total size, and enough for
a person opening it to see the count change as they add to it.

The files here are ordinary. They are not fixtures for the text plugin -
`samples/text/` owns that - they are simply things in a folder.
