# 0.3.0 (Nov 25th, 2025)

* Macro now operates on a module. This allows the macro to have a visibility on both the struct and
  the impl block and would enable us to improve the ergonomics of the API and add new API that's
  currently not possible due to the decoupling between the macro operating on the struct and impl
  block.
* Abstract the user from signal & state change types. Instead provide methods to create streams for
  receiving signals and state changes.
* Published fields can now have a client-side setter method if user asks for it through a new
  sub-attribute, `pub_setter`.
* A few minor fixes in documentation.

# 0.2.0 (Nov 17th, 2025)

* Update info in `Cargo.toml`.
* Port to latest embassy releases.
* Add missing changelog entry for 0.1.1.

# 0.1.1 (Sep 22nd, 2025)

* Add repository link to `Cargo.toml`.

# 0.1.0 (Oct 11th, 2024)

First release. 🎉
