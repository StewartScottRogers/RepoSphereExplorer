# main.bicep

The storage account and queue the readings collector writes to, as an
Azure Bicep template.

Bicep is a language for declaring what should exist rather than what to
do, so what a reader wants from one is the shape of the result: what it
takes in, what it creates, what it hands back. This one takes six
parameters - one of them `@secure()`, one constrained with `@allowed`,
most with defaults - declares four resources across three types, two of
them parented to the first, references two modules, and returns four
outputs.

It also sets `targetScope`, which decides whether the template is
deployed to a resource group, a subscription or a whole tenant. Getting
that wrong is the difference between creating a storage account and
being told you cannot.
