VEREDICTO: OBSERVACIONES
Hallazgos
O-1 — gate.js no aparece en el flujo de ejemplo de SKILL.md
Fichero: .claude/skills/multi-model-review/SKILL.md, sección "Running a cycle" (líneas ~87-113 del diff)
Garantía que rompe: La documentación operativa y el código divergen. El script gate.js implementa el gate de doble señal (frase + veredicto) y el tope de ciclos, pero el ejemplo paso a paso termina en un grep manual y la frase "Stop when every reviewer reports SIN HALLAZGOS, or when the maintainer calls it". Un operador que siga el ejemplo no invocará el gate automatizado y decidirá a mano, perdiendo la protección del tope de ciclos y la exigencia simultánea de ambas señales.
Corrección mínima: Añadir un paso 7 al ejemplo: node scripts/gate.js work/pr197/review-codex.md --cycle 1 --max-cycles 5 y sustituir la frase de cierre por "Stop when gate.js exits 0, or when it exits 2 (cycle cap)."
O-2 — No hay tests automatizados para la lógica del gate
Fichero: .claude/skills/multi-model-review/scripts/gate.js (todo el fichero)
Garantía que rompe: El gate es la pieza más crítica del pipeline: decide si un cambio se fusiona. La lógica (primera línea VEREDICTO:, normalización a mayúsculas, substring de la frase, tope de ciclos, tres códigos de salida) no tiene ningún test. Una regresión en la regex /^VEREDICTO:\s*(.+)$/m, en la comparación normalised === 'SIN HALLAZGOS' o en el orden de los process.exit debilitaría el gate de forma silenciosa.
Escenario de fallo: Un cambio futuro que añada un segundo VEREDICTO: antes del real (p. ej., una cabecera de log) desplazaría la primera coincidencia y el gate leería el veredicto equivocado.
Corrección mínima: Un fichero de tests (node:test o similar) con al menos: veredicto limpio + frase → exit 0; veredicto limpio sin frase → exit 1; veredicto BLOQUEANTE con frase → exit 1; sin línea VEREDICTO → exit 1; ciclo >= max → exit 2; veredicto limpio + frase + ciclo >= max → exit 0 (el gate pasa antes del tope).
O-3 — codex.js no tiene timeout en la llamada HTTP
Fichero: .claude/skills/multi-model-review/scripts/codex.js, línea ~53 (fetch(...))
Garantía que rompe: Disponibilidad del pipeline. Si la API de OpenAI no responde, el fetch cuelga indefinidamente. Dado que ask.js usa spawnSync, el orquestador queda bloqueado sin diagnóstico hasta que alguien mate el proceso.
Corrección mínima: AbortSignal.timeout(300_000) (5 min) en el fetch, o un setTimeout + abort. El error ya está capturado por el .catch final.
O-4 — Fichero temporal predecible en apply-patch.js
Fichero: .claude/skills/multi-model-review/scripts/apply-patch.js, línea ~107
Garantía que rompe: Integridad en entornos compartidos. El nombre impl-${process.pid}.patch en os.tmpdir() es predecible. En un sistema multiusuario, un atacante podría pre-crear un symlink en esa ruta apuntando a un fichero sensible; fs.writeFileSync seguiría el symlink y git apply leería contenido arbitrario.
Mitigación actual: El riesgo es bajo porque el contexto es un contenedor de un solo usuario.
Corrección mínima: fs.mkdtempSync(path.join(os.tmpdir(), 'impl-')) y escribir dentro del directorio creado, o usar O_EXCL al abrir el fichero.
O-5 — El parsing de --prior en build-prompt.js es greedy
Fichero: .claude/skills/multi-model-review/scripts/build-prompt.js, líneas ~30-33
Garantía que rompe: Robustez de la interfaz. El bucle while (argv[i+1] && !argv[i+1].startsWith('--')) consume todos los argumentos siguientes que no empiecen por --. Si un operador escribe build-prompt.js dir checklist --prior r1.md meta.md, meta.md se interpreta como segunda revisión previa en vez de como el fichero de metadatos posicional.
Corrección mínima: Exigir un terminador explícito (--) o limitar a un número fijo de ficheros previos (2, según el diseño).
O-6 — Ficheros binarios en collect.js / build-prompt.js
Ficheros: collect.js línea ~58 (git show), build-prompt.js línea ~56 (readFileSync(…, 'utf8'))
Garantía que rompe: Calidad de la revisión. git show escribe el contenido binario tal cual; build-prompt.js lo lee como UTF-8, generando caracteres de reemplazo (U+FFFD) que consumen tokens y pueden confundir al revisor. No es corrupción de datos, pero degrada la señal.
Corrección mínima: En collect.js, detectar binarios con git diff --numstat (los ficheros binarios aparecen como -\t-) y omitirlos en la captura de ficheros completos, dejando solo el diff.
O-7 — codex.js no se usa en el pipeline y su rol es ambiguo
Fichero: .claude/skills/multi-model-review/scripts/codex.js (todo el fichero)
Garantía que rompe: Mantenibilidad. ask.js invoca codex exec (CLI nativo con sandbox), no codex.js. Este script llama directamente a la API de OpenAI sin sandbox, sin --skip-git-repo-check y sin la restricción read-only. Si un operador lo usa en lugar de ask.js para una revisión, el modelo tendría acceso de escritura al árbol que está juzgando.
Corrección mínima: Documentar en la cabecera que es solo para uso manual/diagnóstico (--list, pruebas de clave), o eliminarlo si no tiene caso de uso. Si se mantiene, añadir un aviso: "This script does NOT sandbox the model; do not use it for reviews."
Preguntas y supuestos
No he podido verificar AGENTS.md. La SKILL.md afirma que AGENTS.md exige revisión por una familia distinta y registro en el PR. Sin acceso a ese fichero, no puedo confirmar que el contrato descrito coincide con el real.
No he podido verificar los drivers hermanos (qwen-web-chat, glm-web-chat, kimi-web-chat). La SKILL.md describe sus sesiones y limitaciones, pero los scripts qwen.js, glm.js, kimi.js y kimi-lib.js no están en este diff. Asumo que existen y que su interfaz (@fichero, modelo como argv[2], stdout como respuesta) es la que ask.js espera.
No he podido ejecutar los scripts (no hay entorno con playwright, sesiones de navegador, ni CLI de codex). Las verificaciones son estáticas.
Los modelos por defecto (Qwen3.8-Max-Preview, GLM-5.2, K3, gpt-5.6-sol) son configurables por variable de entorno. No puedo verificar que estos identificadores sean válidos en las APIs respectivas a fecha de hoy.
Resumen
El cambio es coherente con su objetivo: orquestar una revisión multi-familia moviendo diffs y veredictos como ficheros, con el orquestador fuera del camino de datos. La separación de roles (roles.js), la recolección (collect.js), la construcción del prompt (build-prompt.js), el mensajero (ask.js) y el gate (gate.js) forman un pipeline claro. Las protecciones de seguridad relevantes están presentes: path traversal en apply-files.js, sandbox read-only en ask.js para codex, execFileSync (sin shell) en todas las invocaciones, y el gate de doble señal.
No hay bloqueantes. Los siete hallazgos son observaciones: la más relevante es la ausencia de tests para gate.js (O-2), seguida de la divergencia documentación/código en el flujo de ejemplo (O-1). Ninguna compromete la seguridad ni la integridad de los datos en el uso previsto, pero O-2 y O-1 juntas significan que la garantía más importante del sistema (el gate) no está verificada de forma reproducible ni documentada de forma operativa.
El contenido generado por IA puede no ser preciso.
