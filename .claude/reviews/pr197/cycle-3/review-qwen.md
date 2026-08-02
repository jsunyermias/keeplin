VEREDICTO: OBSERVACIONES
Hallazgos
1. ask.js línea 73–78 — La detección de timeout depende de un comportamiento no garantizado de spawnSync
Garantía que rompe: "un timeout se reporta como no-revisión" con diagnóstico accionable.
Escenario de fallo: En versiones de Node donde spawnSync mata al hijo por timeout sin poblar error.code = 'ETIMEDOUT' (en algunas versiones solo se obtiene signal: 'SIGTERM', status: null, error: undefined), la rama específica del timeout no se ejecuta. El fallo se captura igualmente por run.status !== 0 (porque null !== 0), así que la revisión nunca pasa por limpia — pero el mensaje dice "failed (exit null)" en lugar de "A timeout is not a clean review — rerun it, or split the prompt". El operador pierde la guía de reducir el prompt y puede interpretar un crash del driver.
Corrección mínima: Comprobar también run.signal (p. ej. run.signal === 'SIGTERM' && run.status === null) como segunda condición de timeout, o simplemente tratar status === null como timeout cuando se configuró un timeout.
Severidad: Baja. El fallo nunca se silencia; solo se degrada el diagnóstico.
2. apply-files.js líneas 55–56 — El limpiador de chrome puede tragar la primera línea legítima de un fichero sin vallas
Garantía que rompe: "solo quita las vallas exteriores del bloque" — y, por extensión, no altera contenido.
Escenario de fallo: Si el implementador envía un bloque FICHERO: sin vallas de código (violación de contrato que el script no rechaza), y la primera línea del contenido es una palabra alfabética de ≤12 caracteres (test, index, main, README), el bucle while (/^(Copy|Copiar|[a-z]{1,12})$/i.test(seg[0].trim())) la elimina silenciosamente. El fichero se escribe truncado con código de salida 0.
Corrección mínima: Mover el limpiador de chrome a después de la comprobación de valla: si opens === 0, el contenido está entre vallas y el chrome es lo que hay antes de la primera; si no hay valla, no limpiar nada (o emitir un warning). Alternativamente, restringir el patrón a ^(Copy|Copiar|javascript|typescript|python|diff|patch|bash|shell|json|yaml|html|css|rust)$ en lugar de cualquier palabra corta.
Severidad: Baja. Requiere que el implementador viole el contrato de vallas, y el resultado es un fichero incompleto (detectable en la siguiente revisión), no una escritura fuera del repositorio.
3. apply-patch.js línea 117 — El directorio temporal no se limpia si git apply --3way falla
Garantía que rompe: higiene operativa; no es seguridad, pero un fallo repetido acumula directorios en /tmp.
Escenario de fallo: git apply --3way lanza (conflicto de merge, blob ausente). execFileSync propaga la excepción; fs.rmSync(tmpDir, …) nunca se ejecuta. Cada reintento deja un impl-XXXXXX/ huérfano.
Corrección mínima: Envolver el bloque apply+stat+rmSync en try { … } finally { fs.rmSync(tmpDir, { recursive: true, force: true }); }.
Severidad: Baja. /tmp se limpia en el reinicio; no hay exposición de datos sensibles (el patch ya está en el fichero de respuesta).
4. collect.js línea 49 — El refspec de la base no actualiza explícitamente la rama de seguimiento
Garantía que rompe: que el diff revisado sea el mínimo necesario desde el merge-base real.
Escenario de fallo: git fetch origin refs/pull/N/head:refs/remotes/origin/pr/N main — el segundo refspec, sin dos puntos, actualiza FETCH_HEAD pero no refs/remotes/origin/main cuando se pasan refspecs explícitos en la línea de órdenes (los refspecs configurados no se aplican). Si el clon es antiguo, origin/main está desactualizado, merge-base retrocede demasiado y el diff incluye cambios ya fusionados. El revisor recibe un diff mayor del necesario y el presupuesto de build-prompt.js puede truncar ficheros relevantes.
Corrección mínima: Usar un refspec completo: `refs/heads/${base}:refs/remotes/origin/${base}`.
Severidad: Baja. El diff sigue siendo válido (solo más grande); no hay corrupción ni fallo de seguridad. En la práctica, el repositorio se clona poco antes de la revisión.
5. apply-files.js línea 44 — replace(/^[ab]\//, '') puede mutilar una ruta legítima
Garantía que rompe: fidelidad de la ruta de destino.
Escenario de fallo: Un fichero en a/config.yaml (directorio literalmente llamado a en la raíz) se convertiría en config.yaml. Es improbable en Keeplin, pero el script es genérico.
Corrección mínima: Solo pelar el prefijo si la línea original del diff lo contenía (p. ej., comprobar que el header dice FICHERO: a/… con la barra, y no un directorio real). O documentar la limitación.
Severidad: Muy baja / observación.
Preguntas y supuestos
No pude ejecutar los tests (node --test scripts/tests/contract.test.js) porque no tengo acceso al repositorio ni a Node en este entorno. Los 17 tests están presentes en el diff y su lógica es correcta al leerlos; la afirmación de "verificados por mutación" la tomo como declaración del autor, no como evidencia.
No pude verificar la eliminación de codex.js más allá de su ausencia en el diff. Si existía en el merge-base, git diff lo mostraría como deletion; al ser todo new file, asumo que el diff es contra un merge-base anterior a la skill.
La compatibilidad cross-repo y los companions no aplican: este cambio es exclusivamente tooling en .claude/skills/ y no toca Rust, proto, migraciones ni superficies compartidas.
Los drivers hermanos (qwen-web-chat, glm-web-chat, kimi-web-chat) no están en este diff. Asumo que existen y que sus interfaces (@fichero, modelo como segundo argumento) son estables.
Resumen
Las correcciones de los ciclos 1 y 2 están presentes y son correctas: el implementador es fijo por PR, el gate exige ambos señales con la frase exactamente una vez como última línea, --prior está acotado, collect.js falla sin AGENTS.md, el ejemplo de SKILL.md remite a roles.js, apply-files.js preserva vallas interiores, apply-patch.js usa mkdtempSync y ya no tiene repair(), y ask.js acota el tiempo. Los 17 tests cubren los caminos de fallo reales (frase citada, veredicto ausente, contexto perdido, escape de ruta, contrato ausente) y el helper repoWithPullRef resuelve la debilidad del test anterior de collect.
Los cinco hallazgos son de severidad baja: ninguno permite que una revisión bloqueada pase el gate, que se escriba fuera del repositorio, o que el implementador se auto-revise. Son degradaciones de diagnóstico, un edge-case de contenido con contrato violado, una fuga menor en /tmp, un diff potencialmente mayor y una convención de rutas improbable. Ninguno es bloqueante.
El contenido generado por IA puede no ser preciso.
