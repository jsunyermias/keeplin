# Revisión independiente — PR #197, ciclo 5 (tope alcanzado)

Área: `.claude/skills/multi-model-review/`, resultado integrado desde el
merge-base. El árbol no se modificó mientras los revisores trabajaban.

| Revisor | Modelo | Papel | Veredicto |
| --- | --- | --- | --- |
| Qwen | Qwen3.8-Max-Preview | ciego | OBSERVACIONES |
| GLM | GLM-5.2 | ciego | **BLOQUEANTE** |
| Codex | gpt-5.6-sol | árbitro | **BLOQUEANTE** |

`gate.js --cycle 5 --max-cycles 5` sale con código 2: tope de ciclos alcanzado
con bloqueantes vivos. El contrato dice parar y elevarlo al mantenedor, no
girar una sexta vuelta. Esta revisión se cierra ahí a propósito.

## Bloqueantes, los tres reproducidos antes de escribirlos aquí

No se dan por buenos porque los diga un revisor. Cada uno se midió:

1. **`apply-files.js` sigue escribiendo fuera del repositorio.** La corrección
   del ciclo 4 canonicaliza el ancestro más profundo que existe, y arranca en
   `path.dirname(target)`. Si el destino final *es* el symlink, su padre es
   legítimo y `writeFileSync` sigue el enlace. Reproducido: con
   `repo/link.js -> /tmp/external.js`, un bloque `FICHERO: link.js` sobrescribió
   el fichero externo y el script informó `wrote link.js` como si nada. La
   garantía documentada («no se escribe fuera del repositorio») era falsa para
   el caso más directo. El test del ciclo 4 solo cubría el symlink de
   directorio.

2. **`apply-files.js` rechaza todo cuando el repositorio se alcanza por un
   symlink.** `repoReal` se canonicaliza pero `target` se construye desde
   `REPO` sin canonicalizar, así que nunca empieza por `repoReal`. Reproducido:
   con `workspace -> real/repo`, un fichero nuevo perfectamente legítimo falla
   con `refusing to write outside the repository`. Es el mismo defecto por el
   otro lado: la comprobación ni contiene lo que debe ni deja pasar lo que
   debería.

3. **El procedimiento documentado no activa la protección del ciclo 4.**
   `SKILL.md` paso 5 todavía afirma que «gate.js does not check who replied»
   —falso desde el ciclo 4— y el paso 7 ejecuta el gate sin `--roles`. Quien
   siga el procedimiento oficial deja sin usar exactamente la comprobación de
   independencia que se añadió. Además `roles.js 197` sin `--dir` escribe
   `roles-pr197.json` en el directorio actual, de modo que la persistencia se
   elude cambiando de directorio.

Y dos medios, ambos verificados:

4. **El tope de ciclos se elude con argumentos inválidos.** `--cycle cinco`
   produce `NaN`, `NaN >= NaN` es falso y el gate sale 1 —«sigue iterando»—
   en vez de 2. Reproducido: `--cycle cinco --max-cycles 5` sale 1 con
   `"cycle": null` en el JSON. El contrato promete 2 ante uso incorrecto.

5. **`build-prompt.js` informa de ficheros que no ha incluido.** Cuenta
   `changed.length`, no los que llegó a incrustar. En este mismo ciclo imprimió
   `9 files, 0 omitted` con `files/` vacío: los tres revisores juzgaron el
   cambio solo por el diff, sin el texto de los ficheros, y nadie lo supo
   —salvo Codex, que notó el hueco y lo dijo—. El operador leyó «9 files» como
   confirmación de que el contexto viajaba. Un contador que miente sobre el
   contexto enviado invalida la lectura de la propia revisión.

## Lo que sí quedó resuelto

La persistencia de la asignación y la atribución del revisor (ciclo 4) están
implementadas y el árbitro las da por buenas. El refspec explícito de
`collect.js` se confirma correcto: ningún revisor sostiene ya el hallazgo de
GLM del ciclo 4, que ya se había medido como falso.

## Nota sobre los papeles en este ciclo

`roles.js 197` asigna implementador `codex` y árbitro `kimi`. En la práctica
quien implementó fue Claude, fuera de la rotación, y Codex ha arbitrado. El
gate se ejecutó por tanto **sin** `--roles`: pasárselo habría rechazado a Codex
por no ser el árbitro asignado. Queda escrito en vez de fabricar una asignación
que encajase, porque el propio hueco es un hallazgo: `roles.js` no sabe
expresar «esto lo implementó alguien ajeno a la rotación», y mientras no lo
sepa, la comprobación de independencia no se puede activar en un caso real como
este.
