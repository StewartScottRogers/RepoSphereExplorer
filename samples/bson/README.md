# readings.bson

Four Binary JSON (BSON) documents, one after another, as `mongodump`
writes them.

Each carries an object identifier, a string, a 32-bit and a 64-bit
integer, a date, an array of doubles, an embedded document, a null, and
two binary fields with different subtypes — plain bytes and a universally
unique identifier. The subtype is the part a reader wants told apart: the
same sixteen bytes mean different things under each.

The fourth was annotated by hand afterwards, so it carries two fields
the others have not got. A collection is not a table, and that is the
part which catches out anything treating it as one.
