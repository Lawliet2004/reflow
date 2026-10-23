# Paired installed-model transcription review

> **Latest source:** See [paragraph-preserving corrections](paragraph-preservation/README.md). The measurements below identify an earlier source revision; the newer contract also checks explicit line breaks.

> **Current-source update:** See [the final-source review](final-review.md) and `final-*.json`. The measurements and case comparisons below describe the earlier implementation. The current fixed 50-case set meets 50/50 coarse contracts; naturalness and cased-input limitations remain.

This is a bounded review of 50 fixed text inputs through the production pipeline, using installed Qwen3.5-0.8B Q4_K_M weights. It is not a clean-machine ASR/installer test or proof of universal factual accuracy. Raw measurements, context verification and source/model hashes are in the adjacent JSON files. No model weights were downloaded.

The coarse automated contracts check required/forbidden text fragments and simple list layout. They miss several naturalness problems documented below. A fallback preserves Stage 1 text and is never counted as an applied model rewrite.

At 4096, 40/50 rewrites applied versus 25/50 in the baseline. 13 cases have an identified polish/structure limitation in this review; this includes list punctuation, not just major failures. No new factual addition or meaningful omission was identified in the final 50 text outputs during this bounded review; the contracts alone cannot establish that conclusion.

## Measurements

| Run                    | Applied | Contracts | Median / p95 ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | --------: | --------------: | ------------------: | --------------: |
| Baseline, 4096         |   25/50 |     37/50 |    291.5 / 1516 |                 0.0 |               0 |
| Deterministic, 2048    |   40/50 |     49/50 |     541.5 / 894 |               751.8 |            1238 |
| Deterministic, 4096    |   40/50 |     49/50 |       290 / 414 |               752.1 |            1264 |
| Deterministic, 8192    |   40/50 |     49/50 |       299 / 437 |               752.1 |            1311 |
| Temperature 0.15, 4096 |   40/50 |     49/50 |       315 / 656 |               750.6 |            1264 |

Each deterministic run queried the live `/props` endpoint and asserted the reported per-slot context matched the client budget. 4096 remains the default for new/unset preferences; saved values are preserved. Compared with 4096, 8192 showed equivalent outputs and more VRAM. The 2048 run was measured under changing desktop/compile load and does not establish a causal latency advantage for larger contexts. A repeated isolated CPU/GPU comparison remains useful. Baseline memory was not captured. Timings include deterministic formatting and rewrite work; per-request prompt/decode time and token counts are retained in JSON.

Temperature 0.15 changed only commas in two cases, met the same contracts, and had higher median latency. Deterministic decoding remains. A stronger sentence/list prompt did not resolve either issue and was discarded; its 50 outputs are preserved in `discarded-prompt.json`.

## Case-by-case comparison

### 1. repeated_words

**Input**

```text
so so I think the frontend should should be faster
```

**Before**

```text
So so I think the frontend should be faster.
```

**After**

```text
I think the frontend should be faster.
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: content, relationships, uncertainty, or an unresolved repair changed; original transcript preserved.

### 2. tea_coffee

**Input**

```text
I want tea no no sorry I want coffee
```

**Before**

```text
I want tea no no sorry I want coffee.
```

**After**

```text
I want coffee.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 3. ordinary_paragraph

**Input**

```text
hey I wanted to ask you about the project I finished the frontend yesterday and I'm working on the backend today
```

**Before**

```text
Hey, I wanted to ask you about the project I finished the frontend yesterday and I'm working on the backend today.
```

**After**

```text
Hey, I wanted to ask you about the project I finished the frontend yesterday and I'm working on the backend today.
```

Model rewrite applied. Unresolved run-on after “project”; it needs a sentence boundary.

### 4. independent_requirements

**Input**

```text
the app should work offline use sqlite for storage support dark mode and keep ram usage low
```

**Before**

```text
The app should work offline use sqlite for storage support dark mode and keep ram usage low.
```

**After**

```text
The app should work offline, use SQLite for storage, support dark mode, and keep RAM usage low.
```

Model rewrite applied. Independent requirements remain one sentence; expected bullets are absent.

### 5. sequential_steps

**Input**

```text
first install python then create a virtual environment after that install dependencies and finally start the server
```

**Before**

```text
First install python then create a virtual environment after that install dependencies and finally start the server.
```

**After**

```text
1. Install Python
2. Create a virtual environment
3. Install dependencies
4. Start the server
```

Model rewrite applied. Correct ordered steps and content; terminal punctuation is inconsistent with polished prose.

### 6. genuine_negation

**Input**

```text
do not delete the database no seriously don't delete it
```

**Before**

```text
Do not delete the database no seriously don't delete it.
```

**After**

```text
Do not delete the database. Seriously, don't delete it.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 7. technical_correction

**Input**

```text
use react no actually svelte and connect it to postgres not mysql
```

**Before**

```text
Use react no actually svelte and connect it to postgres not mysql.
```

**After**

```text
Use svelte and connect it to postgres not mysql.
```

Deterministic fallback; model rewrite not applied. Safe fallback resolves the technology choice but leaves casing and missing exclusion punctuation unpolished.

Reported fallback: LLM rewrite rejected: content, relationships, uncertainty, or an unresolved repair changed; original transcript preserved.

### 8. meaningful_very

**Input**

```text
this is very very important
```

**Before**

```text
This is very very important.
```

**After**

```text
This is very, very important.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 9. reported_no

**Input**

```text
he shouted no no no
```

**Before**

```text
He shouted no no no.
```

**After**

```text
He shouted no no no.
```

Model rewrite applied. Emphasis is retained, but reported speech lacks natural quotation/comma punctuation.

### 10. coffee_restart

**Input**

```text
I want to no no no sorry I want coffee
```

**Before**

```text
I want to no no no sorry I want coffee.
```

**After**

```text
I want coffee.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 11. framework_wait

**Input**

```text
Use React, no wait, use Svelte for the frontend.
```

**Before**

```text
Use React, no wait, use Svelte for the frontend.
```

**After**

```text
Use Svelte for the frontend.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 12. deadline

**Input**

```text
The deadline is Monday, sorry, Tuesday.
```

**Before**

```text
The deadline is Tuesday.
```

**After**

```text
The deadline is Tuesday.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 13. multiword_restart

**Input**

```text
I would like to book the morning train no wait I would like to book the evening train
```

**Before**

```text
I would like to book the morning train no wait I would like to book the evening train.
```

**After**

```text
I would like to book the evening train.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 14. multiple_repairs

**Input**

```text
Send it Monday sorry Tuesday and use React no wait use Svelte
```

**Before**

```text
Send it Tuesday and use React no wait use Svelte.
```

**After**

```text
Send it Tuesday and use Svelte.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 15. number_revision

**Input**

```text
order 5 actually 7 notebooks
```

**Before**

```text
Order 7 notebooks.
```

**After**

```text
Order 7 notebooks.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 16. price_literal

**Input**

```text
the price is $125.50 do not change it
```

**Before**

```text
The price is $125.50 do not change it.
```

**After**

```text
The price is $125.50. Do not change it.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 17. quoted_repetition

**Input**

```text
Keep the exact phrase "no no no" in the title.
```

**Before**

```text
Keep the exact phrase "no no no" in the title.
```

**After**

```text
Keep the exact phrase "no no no" in the title.
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: quoted text or literal code changed; original transcript preserved.

### 18. quoted_repair

**Input**

```text
She wrote "Monday sorry Tuesday" on the board.
```

**Before**

```text
She wrote "Monday sorry Tuesday" on the board.
```

**After**

```text
She wrote "Monday sorry Tuesday" on the board.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 19. command

**Input**

```text
run `git reset --soft HEAD~1` but do not run `git reset --hard`
```

**Before**

```text
Run `git reset --soft HEAD~1` but do not run `git reset --hard`.
```

**After**

```text
Run `git reset --soft HEAD~1` but do not run `git reset --hard`.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 20. identifier

**Input**

```text
set retryCount to 3 and keep src/api/client.ts unchanged
```

**Before**

```text
Set retryCount to 3 and keep src/api/client.ts unchanged.
```

**After**

```text
Set retryCount to 3 and keep src/api/client.ts unchanged.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 21. uncertain_plan

**Input**

```text
I think maybe we could release on Friday if the tests pass
```

**Before**

```text
I think maybe we could release on Friday if the tests pass.
```

**After**

```text
I think maybe we could release on Friday if the tests pass.
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: content, relationships, uncertainty, or an unresolved repair changed; original transcript preserved.

### 22. negated_requirement

**Input**

```text
use postgres for storage but do not store passwords in plaintext
```

**Before**

```text
Use PostgreSQL for storage, but do not store passwords in plaintext.
```

**After**

```text
Use PostgreSQL for storage, but do not store passwords in plaintext.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 23. meaningful_had

**Input**

```text
She had had enough of the delays
```

**Before**

```text
She had had enough of the delays.
```

**After**

```text
She had had enough of the delays.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 24. meaningful_that

**Input**

```text
I know that that approach works
```

**Before**

```text
I know that that approach works.
```

**After**

```text
I know that that approach works.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 25. phrase_repeat

**Input**

```text
we could ship we could ship the update tomorrow
```

**Before**

```text
We could ship the update tomorrow.
```

**After**

```text
We could ship the update tomorrow.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 26. question

**Input**

```text
can you send the report before five tomorrow
```

**Before**

```text
Can you send the report before five tomorrow?
```

**After**

```text
Can you send the report before five tomorrow?
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: a number, amount, date, or version changed; original transcript preserved.

### 27. semicolon

**Input**

```text
the train is late semi colon we can wait period
```

**Before**

```text
The train is late; we can wait.
```

**After**

```text
The train is late; we can wait.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 28. spoken_paragraph

**Input**

```text
the north wing is ready period new paragraph the south wing still needs paint period
```

**Before**

```text
The north wing is ready.

The south wing still needs paint.
```

**After**

```text
The north wing is ready.

The south wing still needs paint.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 29. parentheses

**Input**

```text
keep the flag --dry-run (default: false) when testing
```

**Before**

```text
Keep the flag --dry-run (default: false) when testing.
```

**After**

```text
Keep the flag --dry-run (default: false) when testing.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 30. email

**Input**

```text
Hi Noura the invoice is attached thanks Maya
```

**Before**

```text
Hi Noura, the invoice is attached. Thanks, Maya.
```

**After**

```text
Hi Noura, the invoice is attached thanks, Maya.
```

Model rewrite applied. Greeting/sign-off boundaries are not resolved; the placement of Maya is ambiguous.

### 31. requirements_modifiers

**Input**

```text
use encrypted sqlite storage support offline dictation and offer an optional dark mode
```

**Before**

```text
Use encrypted sqlite storage support offline dictation and offer an optional dark mode.
```

**After**

```text
Use encrypted sqlite storage support offline dictation and offer an optional dark mode.
```

Model rewrite applied. Requirements and modifiers remain, but “storage support” is a run-on and separation is inadequate.

### 32. not_sequence

**Input**

```text
first I thought it was broken but then I checked the microphone and it worked
```

**Before**

```text
First I thought it was broken but then I checked the microphone and it worked.
```

**After**

```text
First I thought it was broken but then I checked the microphone and it worked.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 33. meeting_actions

**Input**

```text
Rahul will finish the frontend by Friday Priya owns API integration testing is Saturday and launch is before Monday
```

**Before**

```text
Rahul will finish the frontend by Friday Priya owns API integration testing is Saturday and launch is before Monday.
```

**After**

```text
Rahul will finish the frontend by Friday Priya owns API integration testing is Saturday and launch is before Monday.
```

Deterministic fallback; model rewrite not applied. Safe fallback retains owners/dates, but sentence and action boundaries remain unresolved.

Reported fallback: LLM rewrite rejected: content, relationships, uncertainty, or an unresolved repair changed; original transcript preserved.

### 34. two_topics

**Input**

```text
The frontend is complete and the tests pass. On a different topic the office move is scheduled for Thursday and we still need boxes.
```

**Before**

```text
The frontend is complete and the tests pass. On a different topic, the office move is scheduled for Thursday and we still need boxes.
```

**After**

```text
The frontend is complete and the tests pass. On a different topic, the office move is scheduled for Thursday and we still need boxes.
```

Model rewrite applied. The explicit topic change remains in one paragraph.

### 35. technical_terms

**Input**

```text
connect fast api to postgres and use redis for caching
```

**Before**

```text
Connect fast api to postgres and use redis for caching.
```

**After**

```text
Connect FastAPI to PostgreSQL and use Redis for caching.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 36. first_person

**Input**

```text
I want to build a small application that helps me write without sending audio to the cloud
```

**Before**

```text
I want to build a small application that helps me write without sending audio to the cloud.
```

**After**

```text
I want to build a small application that helps me write without sending audio to the cloud.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 37. quote_code

**Input**

```text
print `print("no no")` exactly and keep the message "actually use React"
```

**Before**

```text
Print `print("no no")` exactly and keep the message "actually use React".
```

**After**

```text
Print `print("no no")` exactly and keep the message "actually use React".
```

Model rewrite applied. No additional issue found in this bounded text review.

### 38. date_version

**Input**

```text
deploy v2.4.1 on 2026-11-03 and use port 8080
```

**Before**

```text
Deploy v2.4.1 on 2026-11-03 and use port 8080.
```

**After**

```text
Deploy v2.4.1 on 2026-11-03 and use port 8080.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 39. no_wait_request

**Input**

```text
No wait for me outside do not leave without me
```

**Before**

```text
No wait for me outside do not leave without me.
```

**After**

```text
No wait for me outside do not leave without me.
```

Model rewrite applied. Words and negation remain, but the opening “No, wait” and second instruction lack clear punctuation.

### 40. actually_emphasis

**Input**

```text
I actually like the current design and I do not want a redesign
```

**Before**

```text
I actually like the current design and I do not want a redesign.
```

**After**

```text
I actually like the current design and I do not want a redesign.
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: content, relationships, uncertainty, or an unresolved repair changed; original transcript preserved.

### 41. rather_preference

**Input**

```text
I would rather keep SQLite than add a remote database
```

**Before**

```text
I would rather keep SQLite than add a remote database.
```

**After**

```text
I would rather keep SQLite than add a remote database.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 42. hinglish

**Input**

```text
Amina ने 2 boxes भेजे लेकिन मुझे 3 नहीं चाहिए
```

**Before**

```text
Amina ने 2 boxes भेजे लेकिन मुझे 3 नहीं चाहिए.
```

**After**

```text
Amina ने 2 boxes भेजे लेकिन मुझे 3 नहीं चाहिए.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 43. hindi

**Input**

```text
मुझे सोमवार को रिपोर्ट भेजनी है और डेटा मिटाना नहीं है
```

**Before**

```text
मुझे सोमवार को रिपोर्ट भेजनी है और डेटा मिटाना नहीं है।
```

**After**

```text
मुझे सोमवार को रिपोर्ट भेजनी है और डेटा मिटाना नहीं है।
```

Model rewrite applied. No additional issue found in this bounded text review.

### 44. spanish

**Input**

```text
no borres la base de datos y conserva los nombres originales
```

**Before**

```text
No borres la base de datos y conserva los nombres originales.
```

**After**

```text
No borres la base de datos y conserva los nombres originales.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 45. technical_negation

**Input**

```text
use Svelte not React and PostgreSQL not MySQL
```

**Before**

```text
Use Svelte not React and PostgreSQL not MySQL.
```

**After**

```text
Use Svelte not React and PostgreSQL not MySQL.
```

Deterministic fallback; model rewrite not applied. No additional issue found in this bounded text review.

Reported fallback: LLM rewrite rejected: a technical identifier, command, path, or name changed; original transcript preserved.

### 46. repeated_entire_clause

**Input**

```text
I need the report I need the report by Friday
```

**Before**

```text
I need the report by Friday.
```

**After**

```text
I need the report by Friday.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 47. uncertain_amount

**Input**

```text
we might need around 20 or 25 licenses depending on who joins
```

**Before**

```text
We might need around 20 or 25 licenses depending on who joins.
```

**After**

```text
We might need around 20 or 25 licenses depending on who joins.
```

Model rewrite applied. No additional issue found in this bounded text review.

### 48. sequence_with_constraint

**Input**

```text
first back up the database then run the migration after that verify the rows and finally restart only if verification passes
```

**Before**

```text
First back up the database then run the migration after that verify the rows and finally restart only if verification passes.
```

**After**

```text
1. Back up the database
2. Run the migration
3. Verify the rows
4. Restart only if verification passes
```

Model rewrite applied. Order and final condition are retained; list items omit terminal punctuation.

### 49. long

**Input**

```text
Um we need to improve the dictation application and keep everything local. The the frontend should show microphone permissions and support dark mode. For the backend use React no wait use Python FastAPI with PostgreSQL and Redis caching. Do not upload audio and do not log passwords. The deadline is Monday sorry Tuesday and Amina owns the API integration.

On the hardware side the target has 16 GB RAM and an RTX 2050 with 4 GB VRAM. We should maybe try a 4096 token context if memory allows but preserve explicit preferences. Keep the command `python -m pytest` exactly and retain the path src/api/client.ts.

For rollout first back up the database then install dependencies then run the tests and finally restart the server only if all checks pass. The release should work offline support encrypted SQLite storage and avoid unnecessary downloads. I want the result to be clear and natural without losing any details.
```

**Before**

```text
We need to improve the dictation application and keep everything local. The frontend should show microphone permissions and support dark mode. For the backend use React no wait use Python FastAPI with PostgreSQL and Redis caching. Do not upload audio and do not log passwords. The deadline is Tuesday and Amina owns the API integration.

On the hardware side the target has 16 GB RAM and an RTX 2050 with 4 GB VRAM. We should maybe try a 4096 token context if memory allows but preserve explicit preferences. Keep the command `python -m pytest` exactly and retain the path src/api/client.ts.

For rollout first back up the database then install dependencies then run the tests and finally restart the server only if all checks pass. The release should work offline support encrypted SQLite storage and avoid unnecessary downloads. I want the result to be clear and natural without losing any details.
```

**After**

```text
We need to improve the dictation application and keep everything local. The frontend should show microphone permissions and support dark mode. For the backend use Python FastAPI with PostgreSQL and Redis caching. Do not upload audio and do not log passwords. The deadline is Tuesday and Amina owns the API integration.

On the hardware side the target has 16 GB RAM and an RTX 2050 with 4 GB VRAM. We should maybe try a 4096 token context if memory allows but preserve explicit preferences. Keep the command `python -m pytest` exactly and retain the path src/api/client.ts.

For rollout first back up the database then install dependencies then run the tests and finally restart the server only if all checks pass. The release should work offline support encrypted SQLite storage and avoid unnecessary downloads. I want the result to be clear and natural without losing any details.
```

Deterministic fallback; model rewrite not applied. Safe fallback retains content and existing paragraphs; embedded rollout/requirements still need coherent structure.

Reported fallback: LLM rewrite rejected: quoted text or literal code changed; original transcript preserved.

### 50. long_corrected_requirements

**Input**

```text
I want to build a privacy focused browser that stays lightweight and uses little RAM. It should open websites search the web and perform tasks only after permission. Maybe we can add local AI assistance later but that is not a launch requirement.

Use Node.js no wait use Python FastAPI for the service and PostgreSQL for the database not MySQL. Store credentials securely and never send private pages to a cloud model. Keep the interface simple with keyboard navigation and a visible cancel action.

Noura will finish the prototype by Thursday sorry Friday. Rahul owns integration tests and Maya will check accessibility. We need Windows macOS and Linux support. Please preserve the setting allowNetwork=false and the exact command `npm run check`. Do not invent additional features or omit any owner or deadline.
```

**Before**

```text
I want to build a privacy focused browser that stays lightweight and uses little RAM. It should open websites search the web and perform tasks only after permission. Maybe we can add local AI assistance later but that is not a launch requirement.

Use Node.js no wait use Python FastAPI for the service and PostgreSQL for the database not MySQL. Store credentials securely and never send private pages to a cloud model. Keep the interface simple with keyboard navigation and a visible cancel action.

Noura will finish the prototype by Friday. Rahul owns integration tests and Maya will check accessibility. We need Windows macOS and Linux support. Please preserve the setting allowNetwork=false and the exact command `npm run check`. Do not invent additional features or omit any owner or deadline.
```

**After**

```text
I want to build a privacy focused browser that stays lightweight and uses little RAM. It should open websites search the web and perform tasks only after permission. Maybe we can add local AI assistance later but that is not a launch requirement. Use Python FastAPI for the service and PostgreSQL for the database not MySQL. Store credentials securely and never send private pages to a cloud model. Keep the interface simple with keyboard navigation and a visible cancel action.

Noura will finish the prototype by Friday. Rahul owns integration tests and Maya will check accessibility. We need Windows macOS and Linux support. Please preserve the setting allowNetwork=false and the exact command `npm run check`. Do not invent additional features or omit any owner or deadline.
```

Deterministic fallback; model rewrite not applied. Safe fallback retains owners, deadlines, literals and requirements; paragraph/requirement structure still needs work.

Reported fallback: LLM rewrite rejected: a technical identifier, command, path, or name changed; original transcript preserved.
