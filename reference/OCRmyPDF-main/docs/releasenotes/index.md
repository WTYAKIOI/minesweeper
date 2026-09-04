% SPDX-FileCopyrightText: 2022 James R. Barlow
% SPDX-License-Identifier: CC-BY-SA-4.0

# Release notes

OCRmyPDF uses [semantic versioning](http://semver.org/) for its
command line interface and its public API.

OCRmyPDF's output messages are not considered part of the stable interface -
that is, output messages may be improved at any release level, so parsing them
may be unreliable. Use the API to depend on precise behavior.

The public API may be useful in scripts that launch OCRmyPDF processes or that
wish to use some of its features for working with PDFs.

The most recent release of OCRmyPDF is ![version](https://img.shields.io/pypi/v/ocrmypdf.svg). Any newer versions
referred to in these notes may exist the main branch but have not been
tagged yet.

OCRmyPDF typically supports the three most recent Python versions.

:::{note}
Attention maintainers: these release notes may be updated with information
about a forthcoming release that has not been tagged yet. A release is only
official when it's tagged and posted to PyPI.
:::

```{toctree}
:glob: true
:maxdepth: 1
:reversed: true

version*
```
