========================
RepoSphereExplorer Guide
========================

:Author: The dark factory
:Version: 0.6.0
:Date: 2026-09-08
:Status: Released

.. contents:: On this page
   :depth: 2
   :local:

.. note::

   Every change in this project starts as a work order. If the guidance
   and the code disagree, the guidance is changed first.

Introduction
============

RepoSphereExplorer is a three-pane file explorer with a plugin per file
type. The service owns the filesystem; the front ends render state and
send intents.

.. warning::

   The service is the only process that writes to disk. A front end that
   touches the filesystem directly is a bug, not a shortcut.

Architecture
============

The three panes
---------------

#. **Folders** - a tree of directories, expandable and drag-targetable.
#. **Contents** - the listing, sortable, multi-selectable, renameable.
#. **File** - supplied entirely by the file-type plugin.

Plugin halves
-------------

Each plugin is one crate with two faces:

Core half
    Linked into the service. Sniffs, parses and extracts. Sees untrusted
    bytes, and runs under the read limits.

Presentation half
    Linked into each front end. Icons, views, edit surface. Never sees a
    raw byte, only the structured data the core produced.

Writing a plugin
================

.. code-block:: rust
   :caption: The smallest useful core half
   :linenos:

   impl PluginCore for MarkdownCore {
       fn name(&self) -> &'static str {
           "markdown"
       }

       fn sniff(&self, prefix: &[u8]) -> bool {
           std::str::from_utf8(prefix)
               .map(|text| text.starts_with("# "))
               .unwrap_or(false)
       }
   }

Then register it, and add a fixture:

.. code-block:: console

   $ ls samples/markdown/
   guide.md
   $ cargo test --all-features

Supported formats
=================

.. list-table:: A sample of the catalogue
   :header-rows: 1
   :widths: 20 20 60

   * - Plugin
     - Extensions
     - What its preview shows
   * - ``rust``
     - ``rs``
     - Traits, structs and functions, then the source
   * - ``image``
     - ``png``, ``jpg``, ``gif``
     - The picture itself, plus dimensions
   * - ``sqlite``
     - ``sqlite``, ``db``
     - Each table's schema and first rows

Reference
=========

.. _work-orders:

Work orders
-----------

A work order states its acceptance checks. The change is finished when
they pass, not when the code looks right. See :ref:`work-orders` for the
canonical wording, and `the issue tracker
<https://github.com/StewartScottRogers/RepoSphereExplorer/issues>`_ for
what is open.

.. seealso::

   ``GUIDANCE.md`` section 3, which this page summarises.

Footnotes
=========

Sniffing is content-based, with the extension as a hint only [#hint]_.

.. [#hint] The hint breaks ties between plugins that all matched, and
   never overrules content on its own.

.. |version| replace:: 0.6.0
.. _upstream: https://github.com/StewartScottRogers/RepoSphereExplorer
