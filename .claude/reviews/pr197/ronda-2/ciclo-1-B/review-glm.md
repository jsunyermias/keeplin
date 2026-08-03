model: GLM-5.2
url: https://chat.z.ai/c/0f3c94c1-b3a0-4468-8472-566d24c7c482
--- reply ---
Show full message
Thought Process
VEREDICTO: BLOQUEANTE
Hallazgos
1. gate.js: --roles es opcional, lo que permite cerrar un ciclo sin verificar la independencia del revisor
Archivo: .claude/skills/multi-model-review/scripts/gate.js (líneas 30-32, 62-93)
Garantía rota: El gate debe exigir que el revisor sea el árbitro asignado y no el implementador. La cadena de documentación y el uso indican que --roles es obligatorio (usage: node gate.js <adjudicator-review.md> --roles <roles-prN.json>), pero el código lo trata como opcional. Si el operador omite --roles, el bloque if (ROLES) se salta por completo y el gate puede devolver 0 (cleared) para una revisión escrita por cualquiera, incluido el implementador.
Escenario de fallo: node gate.js review-codex.md (sin --roles) aprobará la revisión si contiene el veredicto y la frase mágica, sin comprobar quién la escribió ni a qué PR pertenece.
Corrección mínima: Añadir --roles a la lista de argumentos requeridos. Si rolesAt === -1, añadir --roles is required al array bad para que el gate salga con código 2 (uso incorrecto) en lugar de omitir la comprobación.
2. gate.js: El parseo de FILE captura valores de flags como el fichero de revisión
Archivo: .claude/skills/multi-model-review/scripts/gate.js (línea 16)
Garantía rota: El argumento posicional FILE se obtiene con argv.find((a) => !a.startsWith('--')). Esto captura el primer argumento que no empiece por --, incluyendo los valores de otros flags si se pasan antes del fichero.
Escenario de fallo: Si se ejecuta node gate.js --cycle 1 review.md, FILE se resuelve como "1" (el valor de --cycle). El gate intentará leer un fichero llamado 1. Si no existe, saldrá con 2; si existe (por un redireccionamiento accidental), leerá ese en lugar de review.md. Si se ejecuta node gate.js --roles roles.json review.md, FILE será "roles.json", leyendo el JSON de roles como si fuera la revisión. Esto rompe invocaciones válidas y puede llevar a evaluar el fichero equivocado.
Corrección mínima: Filtrar los valores de los flags de la misma manera que lo hace roles.js, excluyendo argv[i+1] para --cycle, --max-cycles y --roles.
3. build-prompt.js: Miente al revisor sobre la causa de los ficheros omitidos y oculta la lista de omitidos
Archivo: .claude/skills/multi-model-review/scripts/build-prompt.js (líneas 181-195)
Garantía rota: Si todos los ficheros exceden el presupuesto de tamaño (FILES_BUDGET_BYTES), filesSection queda vacío. El código entra en el else if (changed.length) y, si NO_FILES es falso, imprime: "No se adjunta el texto completo de ningún fichero: este cambio solo borra ficheros.". Esto es falso: los ficheros no se adjuntaron por tamaño, no porque el PR los borrara. Además, el mensaje que enumera los ficheros omitidos por tamaño está dentro del bloque if (filesSection), por lo que nunca se imprime en este caso.
Escenario de fallo: Un PR que reescriba un único fichero de 200KB. El revisor recibe el mensaje de que "el cambio solo borra ficheros", evaluando el diff sin saber que el contexto completo se omitió por tamaño y sin ver el fichero en la lista de omitidos.
Corrección mínima: Antes de afirmar que el PR solo borra ficheros, comprobar si skipped.length > 0. Si es así, imprimir el mensaje de "Omitidos por tamaño" en lugar del mensaje de borrado.
4. build-prompt.js: Ignora silenciosamente un fichero de objetivo (META) inexistente
Archivo: .claude/skills/multi-model-review/scripts/build-prompt.js (líneas 111-113)
Garantía rota: Si se pasa META como argumento pero el fichero no existe, fs.existsSync(META) es falso y el script construye el prompt sin la sección "Objetivo del cambio", sin lanzar ningún error ni advertencia.
Escenario de fallo: El operador comete un tipográfico en el nombre del fichero de metadatos. El prompt se genera correctamente, pero los revisores juzgan el diff sin conocer el objetivo del cambio, lo que invalida la revisión.
Corrección mínima: Si META se proporciona, exigir que exista. Si no existe, lanzar un error explícito en lugar de omitirlo.
5. ask.js: --pr sin valor se trata como ausente
Archivo: .claude/skills/multi-model-review/scripts/ask.js (líneas 10-11)
Garantía rota: Si --pr es el último argumento, PR es undefined. La validación if (PR !== undefined && !/^\d+$/.test(PR)) no atrapa este caso porque PR ya es undefined. El script continúa y escribe .meta.json sin el campo pr.
Escenario de fallo: node ask.js codex prompt.txt out.md --pr se ejecuta sin error, produciendo un .meta.json sin pr. Aunque gate.js lo rechazará si se usa --roles, fallar tarde en el gate es un mal uso de los recursos (el driver ya se ejecutó).
Corrección mínima: Verificar que si prAt !== -1, argv[prAt + 1] no sea undefined ni empiece por --.
6. build-prompt.js: collect.info malformado omite la validación de paquete
Archivo: .claude/skills/multi-model-review/scripts/build-prompt.js (líneas 74, 102-108)
Garantía rota: Si collect.info existe pero no contiene la línea files_captured:, captured es NaN. La comprobación Number.isInteger(captured) falla y el bloque de validación se salta, aceptando un paquete incompleto.
Corrección mínima: Si collect.info existe pero captured no es un entero, lanzar un error indicando que el paquete está corrupto.
7. roles.js: Sensibilidad a mayúsculas permite asignar implementadores no rotativos inválidos
Archivo: .claude/skills/multi-model-review/scripts/roles.js (líneas 39, 58-60)
Garantía rota: ROTATING.includes(implementer) es sensible a mayúsculas. Si se pasa Kimi como OVERRIDE, se considera externo (external = true), eludiendo la lógica de rotación. Luego, ask.js fallará porque MODELS['Kimi'] no existe.
Corrección mínima: Normalizar OVERRIDE a minúsculas antes de las comprobaciones, o validar contra una lista de agentes conocidos.
8. build-prompt.js: ctxDir con subdirectorios lanza una excepción no controlada
Archivo: .claude/skills/multi-model-review/scripts/build-prompt.js (líneas 129-134)
Garantía rota: fs.readdirSync(ctxDir) devuelve subdirectorios si los hay. read(path.join(ctxDir, f)) lanzará EISDIR al intentar leerlos.
Corrección mínima: Filtrar ctxFiles con f => fs.statSync(path.join(ctxDir, f)).isFile().
Supuestos y limitaciones
No he podido verificar apply-files.js ni collect.js. El prompt indica que la revisión se centra en gate.js, roles.js, ask.js y build-prompt.js, pero la pregunta explícita sobre si apply-files.js puede escribir fuera del repositorio no puedo responderla porque su código no se adjunta. Cualquier riesgo en esa pieza queda fuera de este veredicto.
Asumo que collect.js captura solo los ficheros existentes en files/ y no cuenta los borrados en files_captured, basándome en el comentario de build-prompt.js línea 95. Si collect.js cuenta los ficheros borrados, el hallazgo 6 se convierte en un bloqueante porque la validación embedded + skipped.length !== captured fallará siempre que el PR borre ficheros.
Resumen
El PR corrige varios defectos críticos de la ronda anterior, pero introduce o deja vivos cuatro bloqueantes en el gate y el constructor de prompts. El más grave es que gate.js no exige --roles, lo que permite cerrar un ciclo sin verificar la independencia del revisor, vaciando de contenido la garantía central de esta skill. El parseo de argumentos en gate.js también es defectuoso y puede leer el fichero equivocado. build-prompt.js miente activamente al revisor cuando todos los ficheros exceden el límite de tamaño y omite silenciosamente el objetivo del cambio si el fichero no se encuentra. Estos defectos deben corregirse antes de fusionar.
