# Review di feat/viewer-crate

Note per la revisione del branch. Il file va tolto prima del merge.

Sono 18 commit sopra `main` (4e4e351). Formattazione, clippy con
`-D warnings` e test passano: 52 test, tutti verdi.

```
git log --oneline main..feat/viewer-crate
git diff --stat main...feat/viewer-crate
```

## Cosa c'è

- **Workspace in quattro crate**:
  - `tridi-formats`: i reader, con la sola dipendenza `thiserror`.
  - `tridi-step`: OpenCASCADE in un helper C++ incorporato nel crate ed
    eseguito da memoria (memfd). Se memfd non si può usare, ripiega su una
    copia in `~/.cache/tridi/`, oppure in `tridi-<uid>` nella cartella
    temporanea.
  - `tridi`: il viewer. Si usa con `tridi::show([...])`, oppure con
    `Viewer` e i metodi `.file/.files/.cloud/.theme/.app_id/.output`.
  - `tridi-cli`: il binario `tridi`, con miniature e packaging.
- **Il programma resta quello di prima**: il binario e i pacchetti
  `.deb` e Arch contengono gli stessi file, e le righe su stdout sono
  identiche.
- **Comportamento della libreria**:
  - `show()` si può chiamare più volte nello stesso processo.
  - Quando si chiude una finestra si fermano i suoi caricamenti, e
    l'helper STEP viene ucciso anche a metà file.
  - Le miniature restano fuori dall'API: la feature `thumbnail` è
    nascosta.
- **Pubblicazione**: `cargo publish --workspace --dry-run` impacchetta
  tutti e quattro i crate ed è un controllo in CI. Gli enum pubblici
  sono `#[non_exhaustive]`.
- **Versione**: è ancora 2.8.0. La 3.0.0 è il Task 9, non eseguito.

## Da controllare a mano

Servono un display oppure `sudo`, quindi gli agenti non le hanno potute
fare:

1. **Seconda finestra**: con due `show()` di fila, la seconda deve
   mostrare nella legenda solo il proprio file. Si chiude con `q` o
   `Esc`.
2. **Esempio**:
   `cargo run -p tridi --example show -- tests/data/rgb_ascii.pcd tests/data/assembly.step`.
   La legenda deve avere tre voci, con l'elica colorata come un terzo
   file.
3. **Ripiego in cache**: lancia `sudo sysctl vm.memfd_noexec=2` e apri
   uno STEP. In `~/.cache/tridi/` deve comparire la copia dell'helper.
   Alla fine rimetti il valore a 0.

## Decisioni prese durante il lavoro

Per ognuna c'è il costo nel caso sia sbagliata.

1. Il campo `readme` del crate `tridi` punta al suo README. Costo
   nullo.
2. Un caricamento STEP annullato è sempre `Err(Cancelled)`, anche se
   qualche pezzo era già arrivato. Costo: si perde un conteggio parziale,
   che il viewer comunque non usava.
3. `image` non è più una dipendenza di `tridi`, e `thumbnail = []`.
   Costo: si rimette con una riga.
4. I commit sono di una riga, senza corpo e senza Co-Authored-By.
5. Il Task 9 (3.0.0, tag, `cargo publish`) non è stato eseguito, perché
   una pubblicazione non si annulla.
6. Il ripiego nella cartella temporanea usa `tridi-<uid>`, con permessi
   0700 e un controllo di proprietario e permessi. Prima era `/tmp/tridi`,
   che un altro utente poteva preparare con un eseguibile suo. Costo: un
   ambiente senza memfd e senza `/tmp` utilizzabile perde le miniature
   STEP.
7. Lo stdout di OpenCASCADE non finisce più sul canale dei triangoli.
   L'helper viene sempre terminato e raccolto, e `Pass` è stato tolto.
   Costo: il diff è più grande.
8. I file che non si aprono finiscono sia nel log `warn!` sia in
   `Shown.failed`. Costo: chi usa la libreria vede un avviso in più per
   ogni file.
9. Su "text file busy" (ETXTBSY) l'avvio dell'helper viene ritentato.
   Il problema non si è ripresentato in 20 esecuzioni, quindi la
   correzione è giusta in teoria ma non è provata.
10. Dopo la chiusura della finestra non vengono più inviati pezzi STEP.
    È stata aggiunta una nota su `OtherThread` nella documentazione.
11. Il README non parla di `tridi::init()`, che non è mai stato
    rilasciato. Costo: una frase da aggiungere.
12. Ritocchi alla documentazione, e `send_event` diviso in due `if`.
13. I ritocchi estetici rimandati prima sono finiti nel commit di
    documentazione.
14. `tridi-cli` è pubblicabile grazie a symlink per singolo file in
    `crates/tridi-cli/share/`, che puntano a `share/` alla radice. Il
    dry-run conferma che cargo li segue. Costo: se non funzionasse,
    `cargo install tridi-cli` non compilerebbe.
15. Il README dice che `cargo install` installa solo il binario: niente
    MIME, file `.desktop`, man o completamenti. La sandbox delle
    miniature di Nautilus non vede `$HOME`, quindi per le miniature
    servono i pacchetti.
16. Il comando `sed` in CONTRIBUTING usa i segnaposto `OLD` e `X.Y.Z`.
17. L'ultimo giro di correzioni, dopo la review finale:
    - l'hook dei panic non viene più scambiato (c'era una race);
    - `#[non_exhaustive]` sugli enum pubblici;
    - `NothingOpened { failed }`;
    - il messaggio "cannot load OpenCASCADE";
    - la configurazione docs.rs;
    - `Purpose` e `LoadResult` resi privati;
    - l'rpath solo con `OCCT_LIB_DIR`;
    - i pannelli scuri decisi dalla luminosità;
    - `Debug` e `Clone` su `Cloud` e `Theme`;
    - i punti non finiti scartati in `cloud()`;
    - clippy senza feature in CI.
18. Gli errori di glutin, winit e three-d restano come `#[source]` negli
    errori pubblici. Costo: una major di una di queste librerie obbliga a
    una major di tridi.
19. Le vecchie copie dell'helper in `~/.cache/tridi` non vengono
    cancellate. Costo: qualche MB.

## Problemi minori lasciati aperti

- Un panic di three-d-asset su un `.obj` rotto stampa il suo messaggio
  su stderr.
- `NothingOpened` non ha una catena `#[source]`.
- Il controllo del codice 127 (OCCT mancante) gira solo sul probe.
- Il test del codice 127 richiede `/bin/sh` e una cartella temporanea
  eseguibile.
- Un `OCCT_LIB_DIR` vuoto non è gestito.
- La luminosità dei pannelli si calcola sui colori in spazio gamma.

## Poi

- Scegliere come chiudere il branch: merge in `main` in locale, push con
  PR, oppure lasciarlo com'è.
- Su `main` ci sono due commit della ROADMAP non ancora pushati
  (3eff236, 4e4e351).
- Il Task 9: versione 3.0.0, tag, release su GitHub e
  `cargo publish --workspace`, ogni passo confermato a mano.
