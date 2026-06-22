# Cellos Licensing

## Core System: MPL 2.0

The Cellos kernel and core libraries are licensed under the **Mozilla Public License 2.0 (MPL-2.0)**.

### Covered Components
- **Kernel** (`kernel/`)
- **Core Libraries** (`libs/api`, `libs/ostd`, `libs/types`)
- **HAL** (`hal/`)

### Why MPL 2.0?
MPL 2.0 provides a balanced approach:
- **File-level copyleft**: Modifications to MPL-licensed files must be shared
- **Linking freedom**: Cells can link against MPL code without license restrictions
- **Commercial friendly**: Companies can build proprietary Cells on top of Cellos
- **Patent protection**: Includes explicit patent grants and retaliation clauses

## Cells: Author's Choice

**Cells** (drivers, services, and applications) can use **any license** chosen by their authors:
- Proprietary/closed-source
- MIT, Apache 2.0, BSD
- GPL, LGPL, AGPL
- Or any other license

### Rationale
Cells are independent, dynamically-loaded modules that interact with the kernel only through well-defined API traits. They are:
- **Separate works**: Not derivative of the kernel
- **Dynamically linked**: No static linking to kernel code
- **Interface-based**: Communicate via trait objects, not direct code inclusion

This design ensures that Cell authors have complete freedom over their licensing choices, enabling both open-source collaboration and commercial innovation.

## License Headers

All kernel and libs source files include SPDX license identifiers:

```rust
// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
```

## Full License Text

The complete MPL 2.0 license text is available at:
- https://mozilla.org/MPL/2.0/
- `LICENSE-MPL` (to be added to repository)

## Questions?

For licensing questions, please open an issue in the Cellos repository.
