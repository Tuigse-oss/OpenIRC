# Translations

Each file here is one language of OpenIRC's menus, dialogs, options, and
event lines. The file name is the language code (`de.txt`, `pt-BR.txt`);
Options, Display, Language picks one, or follows the computer's language.

The format is plain text, one entry per line: the English text exactly as
it is in the program, a tab, and the translation.

```text
# A line starting with # and without a tab is a note.
@language<TAB>Deutsch
Quit<TAB>Beenden
{0} joins {1}<TAB>{0} betritt {1}
```

- `{0}`, `{1}`, ... stand for names, numbers, and paths filled in later.
  Keep every one of them; their order may change.
- `\n` is a line break.
- Spaces at the start of a text are part of it (` on {0}`).
- A text with no entry is shown in English.

To add a language, or to try changes without rebuilding, put the file in
the `lang` folder of the OpenIRC settings folder (on Linux
`~/.config/openirc/lang`, on Windows `%APPDATA%\openirc\lang`). A file
there wins over the built-in one with the same code. The test
`every_built_in_catalog_translates_every_text` checks that the built-in
files cover every text in the program and keep every `{0}`.

The built-in translations were made with machine help; corrections from
native speakers are welcome.
