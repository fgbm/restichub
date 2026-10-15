/*
 * ResticHub mockups: the shared model of the profile editor.
 * It draws the field blocks, sums up each section, builds the command preview and
 * draws the retention timeline from static sample data. No network, no storage.
 * Page: profile-a.html. Needs mockups.js for the theme.
 */
(function () {
	'use strict';

	var uid = 0;
	var MAX_DURATION_DEFAULT = 48;
	var STALL_DEFAULT = 60;

	/* ------------------------------------------------------------- sample data */

	var SAMPLE = {
		name: 'srv-web-nightly',
		repo: 's3-prod-primary',
		pathsAll: '/etc/nginx',
		pathsWin: '',
		pathsLinux: '/var/www\n/srv/app/data',
		pathsMac: '',
		excludes: '*.tmp\n/var/www/cache',
		excludeFiles: '/etc/restic/excludes.txt',
		tags: 'web\nnightly',
		vss: false,
		oneFs: true,
		exclCaches: true,
		limUp: '',
		limDown: '',
		keepLast: '3',
		keepHourly: '0',
		keepDaily: '5',
		keepWeekly: '4',
		keepMonthly: '6',
		keepYearly: '0',
		keepWithin: '',
		keepTags: 'before-upgrade',
		gbHost: false,
		gbPaths: false,
		gbTags: false,
		fHosts: '',
		fTags: '',
		fPaths: '',
		subset: '5%',
		stall: '',
		maxDur: '',
		on_excludes: true,
		on_tags: true,
		on_options: true,
		on_retention: true,
		on_check: true,
		on_scripts: true
	};

	var EMPTY = {
		name: '',
		repo: 's3-prod-primary',
		pathsAll: '',
		pathsWin: '',
		pathsLinux: '',
		pathsMac: '',
		excludes: '',
		excludeFiles: '',
		tags: '',
		vss: false,
		oneFs: false,
		exclCaches: false,
		limUp: '',
		limDown: '',
		keepLast: '0',
		keepHourly: '0',
		keepDaily: '7',
		keepWeekly: '4',
		keepMonthly: '6',
		keepYearly: '0',
		keepWithin: '',
		keepTags: '',
		gbHost: false,
		gbPaths: false,
		gbTags: false,
		fHosts: '',
		fTags: '',
		fPaths: '',
		subset: '',
		stall: '',
		maxDur: '',
		on_excludes: false,
		on_tags: false,
		on_options: false,
		on_retention: true,
		on_check: false,
		on_scripts: false,
	};

	var REPOS = ['s3-prod-primary', 'rest-main', 'nas-local', 'azure-archive'];

	var PRESETS = {
		small: { label: 'Минимум', keepLast: '3', keepDaily: '7', keepWeekly: '0', keepMonthly: '0', keepYearly: '0' },
		standard: { label: 'Стандарт', keepLast: '3', keepDaily: '7', keepWeekly: '4', keepMonthly: '6', keepYearly: '0' },
		long: { label: 'Долго', keepLast: '3', keepDaily: '14', keepWeekly: '8', keepMonthly: '12', keepYearly: '5' }
	};

	var RULES = [
		{
			k: 'keepLast',
			unit: 'last',
			title: 'Последние снимки',
			flag: '--keep-last',
			unitText: 'шт.',
			text: 'Оставить N самых новых снимков, какими бы они ни были.'
		},
		{
			k: 'keepHourly',
			unit: 'hourly',
			title: 'По часам',
			flag: '--keep-hourly',
			unitText: 'часов',
			text: 'Взять N последних часов, в которых есть снимки. Из каждого часа остаётся один, самый новый снимок.'
		},
		{
			k: 'keepDaily',
			unit: 'daily',
			title: 'По дням',
			flag: '--keep-daily',
			unitText: 'дней',
			text: 'Взять N последних дней, в которых есть снимки. Из каждого дня остаётся один, самый новый снимок. Дни без снимков не считаются.'
		},
		{
			k: 'keepWeekly',
			unit: 'weekly',
			title: 'По неделям',
			flag: '--keep-weekly',
			unitText: 'недель',
			text: 'Взять N последних недель (с понедельника по воскресенье), в которых есть снимки. Из каждой недели остаётся самый новый снимок.'
		},
		{
			k: 'keepMonthly',
			unit: 'monthly',
			title: 'По месяцам',
			flag: '--keep-monthly',
			unitText: 'месяцев',
			text: 'Взять N последних календарных месяцев, в которых есть снимки. Из каждого месяца остаётся самый новый снимок.'
		},
		{
			k: 'keepYearly',
			unit: 'yearly',
			title: 'По годам',
			flag: '--keep-yearly',
			unitText: 'лет',
			text: 'Взять N последних календарных лет, в которых есть снимки. Из каждого года остаётся самый новый снимок.'
		}
	];

	var SCRIPT_SAMPLES = [
		{
			phase: 'before',
			name: 'Остановить запись в базу',
			template: 'Заморозка PostgreSQL',
			interpreter: 'bash',
			timeout: '300',
			policy: 'abort_backup',
			body: 'psql -c "CHECKPOINT"\npg_ctl -D "$PGDATA" -m fast stop',
			params: [{ name: 'PGDATA', kind: 'string', value: '/var/lib/postgresql/16/main' }]
		},
		{
			phase: 'after',
			name: 'Запустить базу',
			template: 'Заморозка PostgreSQL',
			interpreter: 'bash',
			timeout: '',
			policy: 'continue',
			body: 'pg_ctl -D "$PGDATA" start',
			params: [{ name: 'PGDATA', kind: 'string', value: '/var/lib/postgresql/16/main' }]
		}
	];

	/* ------------------------------------------------------------------ utils */

	function esc(value) {
		return String(value)
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;');
	}

	function lines(value) {
		return String(value || '')
			.split('\n')
			.map(function (line) {
				return line.trim();
			})
			.filter(function (line) {
				return line.length > 0;
			});
	}

	function num(value) {
		var parsed = parseInt(value, 10);
		return isFinite(parsed) && parsed > 0 ? parsed : 0;
	}

	function plural(count, forms) {
		var n = Math.abs(count) % 100;
		var n1 = n % 10;
		if (n > 10 && n < 20) return forms[2];
		if (n1 > 1 && n1 < 5) return forms[1];
		if (n1 === 1) return forms[0];
		return forms[2];
	}

	function icon(name, size) {
		return '<svg class="icon' + (size ? ' icon--' + size : '') + '"><use href="icons.svg#i-' + name + '" /></svg>';
	}

	/* ------------------------------------------------------------ form helpers */

	function text(k, label, value, hint, extra) {
		var id = 'f-' + k + '-' + uid++;
		return (
			'<div class="field"' + (extra && extra.cls ? ' data-field="' + extra.cls + '"' : '') + '>' +
			'<label class="field-label" for="' + id + '">' + label + '</label>' +
			'<input class="input' + (extra && extra.mono ? ' pm-mono' : '') + '" id="' + id + '" data-k="' + k + '" value="' + esc(value) + '"' +
			(extra && extra.placeholder ? ' placeholder="' + esc(extra.placeholder) + '"' : '') +
			(extra && extra.type ? ' inputmode="' + extra.type + '"' : '') +
			' />' +
			(hint ? '<div class="field-hint">' + hint + '</div>' : '') +
			'</div>'
		);
	}

	function area(k, label, value, hint, rows) {
		var id = 'f-' + k + '-' + uid++;
		return (
			'<div class="field">' +
			'<label class="field-label" for="' + id + '">' + label + '</label>' +
			'<textarea class="textarea pm-mono" id="' + id + '" rows="' + (rows || 3) + '" data-k="' + k + '">' + esc(value) + '</textarea>' +
			(hint ? '<div class="field-hint">' + hint + '</div>' : '') +
			'</div>'
		);
	}

	function chips(k, label, value, hint) {
		var id = 'f-' + k + '-' + uid++;
		return (
			'<div class="field"><label class="field-label" for="' + id + '">' + label + '</label>' +
			'<div class="pm-chipbox" data-chipbox>' +
			'<span class="pm-chiplist" data-chiplist></span>' +
			'<input class="pm-chipinput" id="' + id + '" placeholder="Добавьте тег и нажмите Enter" autocomplete="off" />' +
			'<input type="hidden" data-k="' + k + '" value="' + esc(lines(value).join('\n')) + '" />' +
			'</div>' +
			(hint ? '<div class="field-hint">' + hint + '</div>' : '') +
			'</div>'
		);
	}

	function drawChips(box) {
		var hidden = box.querySelector('input[type="hidden"]');
		var list = box.querySelector('[data-chiplist]');
		list.innerHTML = lines(hidden.value).map(function (tag, i) {
			return '<span class="pm-chip"><span>' + esc(tag) + '</span><button type="button" data-chip-remove="' + i + '" aria-label="Убрать тег ' + esc(tag) + '">' + icon('x', 'xs') + '</button></span>';
		}).join('');
	}

	function setChips(box, values) {
		var hidden = box.querySelector('input[type="hidden"]');
		hidden.value = values.join('\n');
		drawChips(box);
		hidden.dispatchEvent(new Event('input', { bubbles: true }));
	}

	function commitChip(box) {
		var input = box.querySelector('.pm-chipinput');
		var parts = input.value.split(/[\s,]+/).filter(Boolean);
		input.value = '';
		if (!parts.length) return;
		var current = lines(box.querySelector('input[type="hidden"]').value);
		parts.forEach(function (part) {
			if (current.indexOf(part) < 0) current.push(part);
		});
		setChips(box, current);
	}

	function check(k, label, checked, hint) {
		return (
			'<div class="pm-check">' +
			'<label class="checkbox"><input type="checkbox" data-k="' + k + '"' + (checked ? ' checked' : '') + ' /><span>' + label + '</span></label>' +
			(hint ? '<div class="field-hint">' + hint + '</div>' : '') +
			'</div>'
		);
	}

	function select(k, label, options, value) {
		var id = 'f-' + k + '-' + uid++;
		return (
			'<div class="field"><label class="field-label" for="' + id + '">' + label + '</label>' +
			'<span class="select-wrap"><select class="select" id="' + id + '" data-k="' + k + '">' +
			options
				.map(function (option) {
					var val = Array.isArray(option) ? option[0] : option;
					var lab = Array.isArray(option) ? option[1] : option;
					return '<option value="' + esc(val) + '"' + (val === value ? ' selected' : '') + '>' + esc(lab) + '</option>';
				})
				.join('') +
			'</select>' + icon('chevron-down') + '</span></div>'
		);
	}

	/* ----------------------------------------------------------------- blocks */

	var BLOCKS = {
		basics: function (s) {
			return (
				'<div class="pm-grid2">' +
				text('name', 'Имя профиля<span class="req">*</span>', s.name, 'Короткое имя, по нему профиль находят в списке и в планах.') +
				select('repo', 'Репозиторий<span class="req">*</span>', REPOS, s.repo) +
				'</div>'
			);
		},
		paths: function (s) {
			return (
				'<p class="pm-lead">Что копировать. По одному пути в строке. Пути «Для всех систем» берутся на любом хосте, остальные только на своей системе.</p>' +
				area('pathsAll', 'Для всех систем', s.pathsAll, '', 2) +
				'<div class="pm-grid3">' +
				area('pathsWin', 'Windows', s.pathsWin, '', 3) +
				area('pathsLinux', 'Linux', s.pathsLinux, '', 3) +
				area('pathsMac', 'macOS', s.pathsMac, '', 3) +
				'</div>'
			);
		},
		excludes: function (s) {
			return (
				'<p class="pm-lead">Что пропустить. Шаблоны вида <span class="code">*.tmp</span> или пути. По одной записи в строке.</p>' +
				'<div class="pm-grid2">' +
				area('excludes', 'Исключения', s.excludes, '', 3) +
				area('excludeFiles', 'Файлы исключений', s.excludeFiles, 'Путь к файлу со списком шаблонов на хосте.', 3) +
				'</div>'
			);
		},
		tags: function (s) {
			return (
				'<p class="pm-lead">Метки снимков этого профиля. По ним снимки ищут и отбирают правила хранения.</p>' +
				chips('tags', 'Теги', s.tags, 'Через запятую, пробел или Enter. Каждый тег станет чипом.')
			);
		},
		options: function (s) {
			return (
				'<div class="pm-checks">' +
				check('vss', 'Снимок файловой системы (VSS)', s.vss, 'Только Windows: копия согласованного снимка тома.') +
				check('oneFs', 'Не выходить за файловую систему', s.oneFs, 'Подключённые тома внутри путей пропускаются.') +
				check('exclCaches', 'Исключить каталоги кэша', s.exclCaches, 'Пропускаются каталоги с файлом CACHEDIR.TAG.') +
				'</div>' +
				'<div class="pm-grid2">' +
				text('limUp', 'Ограничение отдачи, КиБ/с', s.limUp, '', { placeholder: 'Без ограничения', type: 'numeric' }) +
				text('limDown', 'Ограничение приёма, КиБ/с', s.limDown, '', { placeholder: 'Без ограничения', type: 'numeric' }) +
				'</div>'
			);
		},
		check: function (s) {
			return (
				'<p class="pm-lead">Проверка целостности репозитория после копирования. Без доли чтения проверяется только структура: это быстро.</p>' +
				text('subset', 'Доля чтения данных', s.subset, 'Пример: <span class="code">5%</span> или <span class="code">1/10</span>. Чем больше доля, тем дольше проверка и тем больше трафика.', { placeholder: 'Только структура', mono: true })
			);
		},
		limits: function (s) {
			return (
				'<p class="pm-lead">Защита от зависших и слишком долгих заданий. Лимиты действуют всегда: если поле пустое, берётся значение по умолчанию.</p>' +
				'<div class="pm-grid2">' +
				text('stall', 'Остановить, если нет прогресса, минут', s.stall, 'Если агент не сообщает о прогрессе дольше этого срока, задание останавливается как зависшее. По умолчанию <b>60 минут</b>; значение для всего стека задаёт переменная окружения <span class="code">RESTICHUB_JOB_STALL_TIMEOUT_SECS</span>.', { placeholder: '60 (по умолчанию)', type: 'numeric' }) +
				text('maxDur', 'Максимальная длительность, часов', s.maxDur, 'Задание, которое идёт дольше, останавливается, даже если прогресс есть. По умолчанию <b>48 часов</b>; значение по умолчанию для всего стека задаёт переменная окружения <span class="code">RESTICHUB_JOB_MAX_DURATION_SECS</span>.', { placeholder: '48 (по умолчанию)', type: 'numeric' }) +
				'</div>'
			);
		},
		repo: function (s) {
			return select('repo', 'Репозиторий<span class="req">*</span>', REPOS, s.repo);
		},
		scripts: function () {
			return (
				'<div class="alert pm-note">' + icon('lock', 'xs') +
				'<span>Скрипты выполняются на хосте от имени агента и могут менять данные. Править их может только администратор. Хост должен включить режим доверия.</span></div>' +
				'<div class="pm-script-group"><div class="pm-group-title">До резервного копирования</div><div data-script-list="before"></div></div>' +
				'<div class="pm-script-group"><div class="pm-group-title">После резервного копирования</div><div data-script-list="after"></div></div>' +
				'<div class="pm-row-actions">' +
				'<button class="btn btn--sm" type="button" data-script-add>' + icon('plus', 'xs') + ' Пустой скрипт</button>' +
				'<button class="btn btn--sm" type="button">' + icon('book', 'xs') + ' Из шаблона скрипта</button>' +
				'<button class="btn btn--sm" type="button">' + icon('checklist', 'xs') + ' Взять шаблон задания</button>' +
				'</div>'
			);
		},
		retention: function (s) {
			var rows = RULES.map(function (rule) {
				return (
					'<div class="pm-rule">' +
					'<div class="pm-rule-head"><span class="pm-rule-title">' + rule.title + '</span><span class="code pm-flag">' + rule.flag + '</span></div>' +
					'<div class="pm-rule-input"><input class="input pm-num" inputmode="numeric" data-k="' + rule.k + '" value="' + esc(s[rule.k]) + '" aria-label="' + rule.title + '" /><span class="pm-unit">' + rule.unitText + '</span></div>' +
					'<div class="pm-rule-text">' + rule.text + '</div>' +
					'<div class="pm-rule-count" data-rule-count="' + rule.unit + '"></div>' +
					'</div>'
				);
			}).join('');
			var within =
				'<div class="pm-rule">' +
				'<div class="pm-rule-head"><span class="pm-rule-title">Все снимки за период</span><span class="code pm-flag">--keep-within</span></div>' +
				'<div class="pm-rule-input"><input class="input pm-num pm-num--wide pm-mono" data-k="keepWithin" value="' + esc(s.keepWithin) + '" placeholder="30d" aria-label="Период" /><span class="pm-unit">y m d h</span></div>' +
				'<div class="pm-rule-text">Оставить все снимки, которые новее самого нового снимка не больше чем на этот срок. Отсчёт идёт от самого нового снимка, а не от сегодняшнего дня. Пример: <span class="code">30d</span> — 30 дней, <span class="code">1y2m</span> — год и два месяца.</div>' +
				'<div class="pm-rule-count" data-rule-count="within"></div>' +
				'<div class="field-error pm-within-error" data-within-error hidden>' + icon('alert', 'xs') + ' Не разобрать срок. Пример: 30d, 6m, 1y2m7d3h.</div>' +
				'</div>';
			var tags =
				'<div class="pm-rule pm-rule--wide">' +
				'<div class="pm-rule-head"><span class="pm-rule-title">Теги, которые не удаляются</span><span class="code pm-flag">--keep-tag</span></div>' +
				'<div class="pm-rule-text">Снимок с любым из этих тегов остаётся всегда, что бы ни решили остальные правила. Вводите теги через запятую или пробел: каждый станет чипом.</div>' +
				chips('keepTags', 'Теги', s.keepTags, '') +
				'<div class="pm-rule-count" data-rule-count="tags"></div>' +
				'</div>';
			var group =
				'<div class="pm-rule pm-rule--wide">' +
				'<div class="pm-rule-head"><span class="pm-rule-title">Что считать одной группой</span><span class="code pm-flag">--group-by</span></div>' +
				'<div class="pm-rule-text">Правила применяются к каждой группе снимков отдельно, поэтому копии разных хостов или путей не вытесняют друг друга. Если ничего не отмечено, используется группировка по умолчанию: хост и пути.</div>' +
				'<div class="pm-inline-checks">' +
				check('gbHost', 'Хост', s.gbHost) + check('gbPaths', 'Пути', s.gbPaths) + check('gbTags', 'Теги', s.gbTags) +
				'</div></div>';
			var filter =
				'<details class="pm-filter"><summary>' + icon('filter', 'xs') + ' Ограничить правила отдельными снимками <span class="pm-sub" data-filter-sum>— все снимки</span></summary>' +
				'<div class="pm-filter-body"><p class="pm-lead">Правила трогают только снимки, подходящие под фильтр. Остальные не удаляются. Пусто — подходят все.</p>' +
				'<div class="pm-grid3">' +
				area('fHosts', 'Хосты', s.fHosts, 'Один в строке.', 2) +
				chips('fTags', 'Теги', s.fTags, '') +
				area('fPaths', 'Пути', s.fPaths, 'Один в строке.', 2) +
				'</div></div></details>';
			var presets =
				'<div class="pm-presets"><span class="pm-sub">Готовые наборы:</span>' +
				Object.keys(PRESETS)
					.map(function (key) {
						return '<button type="button" class="btn btn--sm" data-preset="' + key + '">' + PRESETS[key].label + '</button>';
					})
					.join('') +
				'</div>';
			var intro =
				'<div class="pm-howto"><div class="pm-howto-title">Как работают правила</div><ul>' +
				'<li>Снимок остаётся, если его оставило <b>хотя бы одно</b> правило. Правила не вычитаются, а складываются.</li>' +
				'<li><b>0 или пустое поле — правило выключено.</b> Если правил нет, ничего не удаляется и очистка (prune) не запускается.</li>' +
				'<li>Правила по часам, дням, неделям, месяцам и годам считают только те интервалы, <b>в которых есть снимки</b>. «5 дней» — это пять дней с копиями, а не последние пять суток.</li>' +
				'</ul></div>';
			var preview =
				'<div class="pm-preview"><div class="pm-preview-head"><span class="pm-group-title">Что останется</span><span class="pm-sub">Пример: ежедневные копии за 60 дней, по две за последние три дня</span></div>' +
				'<div class="pm-timeline" data-timeline role="img" aria-label="Снимки в примере: оставленные и удаляемые"></div>' +
				'<div class="pm-axis"><span>60 дней назад</span><span>30 дней назад</span><span>сегодня</span></div>' +
				'<div class="pm-legend"><span><i class="pm-snap is-kept"></i> остаётся</span><span><i class="pm-snap"></i> будет удалён</span><span><i class="pm-snap is-kept is-tagged"></i> остаётся из-за тега</span></div>' +
				'<div class="pm-keepline" data-keep-line></div></div>';
			return intro + presets + '<div class="pm-rules">' + rows + within + '</div>' + tags + group + filter + preview;
		}
	};

	function drawBlocks(root, state) {
		var nodes = root.querySelectorAll('[data-block]');
		for (var i = 0; i < nodes.length; i++) {
			var name = nodes[i].getAttribute('data-block');
			if (BLOCKS[name]) nodes[i].innerHTML = BLOCKS[name](state);
		}
		var scriptsHost = root.querySelectorAll('[data-script-list]');
		if (scriptsHost.length && state === SAMPLE) {
			SCRIPT_SAMPLES.forEach(function (script) {
				addScript(root, script);
			});
		}
	}

	/* ---------------------------------------------------------------- scripts */

	function scriptHtml(script) {
		var id = 'sc-' + uid++;
		var phaseOptions = [['before', 'До резервного копирования'], ['after', 'После резервного копирования']];
		var interps = [['bash', 'Bash'], ['sh', 'sh'], ['powershell', 'PowerShell'], ['cmd', 'cmd']];
		var policies = [['abort_backup', 'Остановить резервное копирование'], ['continue', 'Продолжить и сообщить предупреждение']];
		function sel(label, opts, value, attr) {
			return (
				'<div class="field"><label class="field-label">' + label + '</label><span class="select-wrap"><select class="select" ' + attr + '>' +
				opts.map(function (o) {
					return '<option value="' + o[0] + '"' + (o[0] === value ? ' selected' : '') + '>' + o[1] + '</option>';
				}).join('') + '</select>' + icon('chevron-down') + '</span></div>'
			);
		}
		var params = (script.params || [])
			.map(function (p) {
				return (
					'<div class="pm-param"><input class="input pm-mono" value="' + esc(p.name) + '" aria-label="Переменная" /><input class="input" value="' + esc(p.value) + '" aria-label="Значение" /><button type="button" class="btn btn--icon btn--sm btn--ghost" aria-label="Удалить параметр" data-param-remove>' + icon('x', 'xs') + '</button></div>'
				);
			})
			.join('');
		return (
			'<div class="pm-script" data-script data-phase="' + script.phase + '" data-name="' + esc(script.name) + '">' +
			'<div class="pm-script-head">' +
			'<button type="button" class="pm-script-toggle" aria-expanded="false" aria-controls="' + id + '">' +
			icon('chevron-right', 'xs') +
			'<span class="pm-script-name">' + esc(script.name) + '</span>' +
			(script.template ? '<span class="label">Шаблон: ' + esc(script.template) + '</span>' : '') +
			'<span class="pm-sub">' + esc(script.interpreter) + (script.timeout ? ' · до ' + script.timeout + ' с' : ' · до 1 часа') + (script.policy === 'continue' ? ' · при ошибке продолжить' : ' · при ошибке остановить') + '</span>' +
			'</button>' +
			'<button type="button" class="btn btn--icon btn--sm btn--ghost" aria-label="Переместить вверх">' + icon('chevron-up', 'xs') + '</button>' +
			'<button type="button" class="btn btn--icon btn--sm btn--ghost" aria-label="Удалить скрипт" data-script-remove>' + icon('trash', 'xs') + '</button>' +
			'</div>' +
			'<div class="pm-script-body" id="' + id + '" hidden>' +
			'<div class="pm-grid2">' +
			'<div class="field"><label class="field-label">Название</label><input class="input" value="' + esc(script.name) + '" data-script-name /></div>' +
			sel('Когда выполняется', phaseOptions, script.phase, 'data-script-phase-select') +
			sel('Интерпретатор', interps, script.interpreter, '') +
			'<div class="field"><label class="field-label">Лимит времени, секунд</label><input class="input" value="' + esc(script.timeout) + '" placeholder="1 час" /><div class="field-hint">Пустое значение означает один час.</div></div>' +
			'</div>' +
			sel('Если скрипт завершился с ошибкой', policies, script.policy, '') +
			'<div class="field"><label class="field-label">Скрипт</label><textarea class="textarea pm-mono" rows="4">' + esc(script.body) + '</textarea><div class="field-hint">Текст скрипта. Не больше 64 КиБ.</div></div>' +
			'<div class="field"><label class="field-label">Параметры</label><div class="field-hint">Значения попадают в переменные окружения. Секрет подставляется на хосте и не виден в журнале.</div>' + params +
			'<div class="pm-row-actions"><button type="button" class="btn btn--sm">' + icon('plus', 'xs') + ' Добавить параметр</button></div></div>' +
			'</div></div>'
		);
	}

	function addScript(root, script) {
		var target = root.querySelector('[data-script-list="' + script.phase + '"]');
		if (!target) return;
		var wrap = document.createElement('div');
		wrap.innerHTML = scriptHtml(script);
		target.appendChild(wrap.firstChild);
	}

	/* ------------------------------------------------------------------ state */

	function read(scope) {
		var state = {};
		var nodes = scope.querySelectorAll('[data-k]');
		for (var i = 0; i < nodes.length; i++) {
			var key = nodes[i].getAttribute('data-k');
			state[key] = nodes[i].type === 'checkbox' ? nodes[i].checked : nodes[i].value;
		}
		state.scriptsBefore = scope.querySelectorAll('[data-script][data-phase="before"]').length;
		state.scriptsAfter = scope.querySelectorAll('[data-script][data-phase="after"]').length;
		return state;
	}

	function isOn(state, key) {
		return state[key] === undefined ? true : !!state[key];
	}

	/* -------------------------------------------------------------- retention */

	function sampleSnapshots() {
		var newestDay = Date.UTC(2026, 9, 15, 2, 0);
		var list = [];
		for (var d = 0; d < 60; d++) {
			var t = newestDay - d * 86400000;
			var snap = { t: t, tags: ['web', 'nightly'] };
			if (d === 41) snap.tags = ['web', 'nightly', 'before-upgrade'];
			list.push(snap);
			if (d < 3) list.push({ t: t + 12 * 3600000, tags: ['web', 'nightly'] });
		}
		list.sort(function (a, b) {
			return b.t - a.t;
		});
		return list;
	}

	var SNAPS = sampleSnapshots();

	function bucket(unit, t) {
		var d = new Date(t);
		switch (unit) {
			case 'hourly':
				return Math.floor(t / 3600000);
			case 'daily':
				return Math.floor(t / 86400000);
			case 'weekly':
				return Math.floor((Math.floor(t / 86400000) + 3) / 7);
			case 'monthly':
				return d.getUTCFullYear() * 12 + d.getUTCMonth();
			default:
				return d.getUTCFullYear();
		}
	}

	function pickBuckets(unit, n) {
		var kept = {};
		var left = n;
		var last = null;
		for (var i = 0; i < SNAPS.length && left > 0; i++) {
			var key = bucket(unit, SNAPS[i].t);
			if (key !== last) {
				last = key;
				kept[i] = true;
				left--;
			}
		}
		return kept;
	}

	function parseWithin(value) {
		var match = /^(?:(\d+)y)?(?:(\d+)m)?(?:(\d+)d)?(?:(\d+)h)?$/.exec(String(value).trim());
		if (!match || !String(value).trim()) return null;
		return { y: +(match[1] || 0), m: +(match[2] || 0), d: +(match[3] || 0), h: +(match[4] || 0) };
	}

	function pickWithin(value) {
		var span = parseWithin(value);
		var kept = {};
		if (!span) return kept;
		var latest = new Date(SNAPS[0].t);
		var cutoff = Date.UTC(
			latest.getUTCFullYear() - span.y,
			latest.getUTCMonth() - span.m,
			latest.getUTCDate() - span.d,
			latest.getUTCHours() - span.h,
			latest.getUTCMinutes()
		);
		for (var i = 0; i < SNAPS.length; i++) if (SNAPS[i].t > cutoff) kept[i] = true;
		return kept;
	}

	function pickTags(value) {
		var kept = {};
		var sets = lines(value).map(function (line) {
			return line.split(',').map(function (tag) {
				return tag.trim();
			}).filter(Boolean);
		});
		for (var i = 0; i < SNAPS.length; i++) {
			for (var j = 0; j < sets.length; j++) {
				var all = sets[j].length > 0 && sets[j].every(function (tag) {
					return SNAPS[i].tags.indexOf(tag) >= 0;
				});
				if (all) kept[i] = true;
			}
		}
		return kept;
	}

	function evaluate(state) {
		var per = {};
		var n = num(state.keepLast);
		per.last = {};
		for (var i = 0; i < n && i < SNAPS.length; i++) per.last[i] = true;
		RULES.slice(1).forEach(function (rule) {
			per[rule.unit] = pickBuckets(rule.unit, num(state[rule.k]));
		});
		per.within = pickWithin(state.keepWithin || '');
		per.tags = pickTags(state.keepTags || '');
		var union = {};
		Object.keys(per).forEach(function (key) {
			Object.keys(per[key]).forEach(function (index) {
				union[index] = true;
			});
		});
		return { per: per, union: union };
	}

	function retentionHasRules(state) {
		return (
			RULES.some(function (rule) {
				return num(state[rule.k]) > 0;
			}) || String(state.keepWithin || '').trim() !== '' || lines(state.keepTags).length > 0
		);
	}

	function retentionSummary(state) {
		var parts = [];
		if (num(state.keepLast)) parts.push('последние ' + num(state.keepLast));
		if (num(state.keepHourly)) parts.push('по часам ' + num(state.keepHourly));
		if (num(state.keepDaily)) parts.push('по дням ' + num(state.keepDaily));
		if (num(state.keepWeekly)) parts.push('по неделям ' + num(state.keepWeekly));
		if (num(state.keepMonthly)) parts.push('по месяцам ' + num(state.keepMonthly));
		if (num(state.keepYearly)) parts.push('по годам ' + num(state.keepYearly));
		if (String(state.keepWithin || '').trim()) parts.push('всё за ' + String(state.keepWithin).trim());
		var tags = lines(state.keepTags);
		if (tags.length) parts.push('теги: ' + tags.join(', '));
		return parts;
	}

	function drawRetention(scope, state) {
		var timeline = scope.querySelector('[data-timeline]');
		var result = evaluate(state);
		var active = isOn(state, 'on_retention');
		var hasRules = retentionHasRules(state);
		var withinBad = String(state.keepWithin || '').trim() !== '' && !parseWithin(state.keepWithin);
		var err = scope.querySelector('[data-within-error]');
		if (err) err.hidden = !withinBad;
		var counts = scope.querySelectorAll('[data-rule-count]');
		for (var c = 0; c < counts.length; c++) {
			var key = counts[c].getAttribute('data-rule-count');
			var n = Object.keys(result.per[key] || {}).length;
			counts[c].innerHTML = n ? 'В примере оставит: <b>' + n + '</b>' : '<span class="pm-off">Выключено</span>';
		}
		var filterSum = scope.querySelector('[data-filter-sum]');
		if (filterSum) {
			var f = [];
			if (lines(state.fHosts).length) f.push('хосты: ' + lines(state.fHosts).length);
			if (lines(state.fTags).length) f.push('теги: ' + lines(state.fTags).length);
			if (lines(state.fPaths).length) f.push('пути: ' + lines(state.fPaths).length);
			filterSum.textContent = f.length ? '— ' + f.join(', ') : '— все снимки';
		}
		if (!timeline) return;
		var keepAll = !active || !hasRules;
		var kept = 0;
		var html = '';
		for (var i = SNAPS.length - 1; i >= 0; i--) {
			var isKept = keepAll || result.union[i];
			var tagged = !keepAll && result.per.tags[i];
			if (isKept) kept++;
			var date = new Date(SNAPS[i].t);
			var label = ('0' + date.getUTCDate()).slice(-2) + '.' + ('0' + (date.getUTCMonth() + 1)).slice(-2) + ' ' + ('0' + date.getUTCHours()).slice(-2) + ':00';
			html += '<i class="pm-snap' + (isKept ? ' is-kept' : '') + (tagged ? ' is-tagged' : '') + '" title="' + label + (isKept ? ' — остаётся' : ' — будет удалён') + '"></i>';
		}
		timeline.innerHTML = html;
		var line = scope.querySelector('[data-keep-line]');
		if (line) {
			if (!active) {
				line.innerHTML = 'Хранение выключено: снимки не удаляются. В примере остаются все <b>' + SNAPS.length + '</b>.';
			} else if (!hasRules) {
				line.innerHTML = 'Правил нет, ничего не удаляется. В примере остаются все <b>' + SNAPS.length + '</b>.';
			} else {
				line.innerHTML =
					'Сейчас правило оставит примерно <b>' + kept + '</b> ' + plural(kept, ['снимок', 'снимка', 'снимков']) + ' из ' + SNAPS.length + ' в примере. Остальные ' + (SNAPS.length - kept) + ' будут удалены.';
			}
		}
		var cards = scope.querySelectorAll('[data-keep-total]');
		for (var k = 0; k < cards.length; k++) cards[k].textContent = String(kept);
	}

	/* ------------------------------------------------------------- summaries */

	function summarize(state) {
		var out = {};
		out.basics = { on: !!(state.name && state.name.trim()), text: (state.name || 'без имени') + ' · ' + (state.repo || 'репозиторий не выбран') };
		var counts = [];
		var all = lines(state.pathsAll).length;
		if (all) counts.push('все системы: ' + all);
		if (lines(state.pathsWin).length) counts.push('Windows: ' + lines(state.pathsWin).length);
		if (lines(state.pathsLinux).length) counts.push('Linux: ' + lines(state.pathsLinux).length);
		if (lines(state.pathsMac).length) counts.push('macOS: ' + lines(state.pathsMac).length);
		out.paths = { on: counts.length > 0, text: counts.length ? counts.join(' · ') : 'Пути не заданы', warn: counts.length === 0 };

		var ex = lines(state.excludes).length;
		var exf = lines(state.excludeFiles).length;
		var exParts = [];
		if (ex) exParts.push(ex + ' ' + plural(ex, ['шаблон', 'шаблона', 'шаблонов']));
		if (exf) exParts.push(exf + ' ' + plural(exf, ['файл', 'файла', 'файлов']) + ' исключений');
		out.excludes = off(state, 'on_excludes', exParts.length > 0, exParts.join(' · '), 'Ничего не исключается');

		var tg = lines(state.tags);
		out.tags = off(state, 'on_tags', tg.length > 0, tg.join(' · '), 'Без тегов');

		var op = [];
		if (state.vss) op.push('VSS');
		if (state.oneFs) op.push('не выходить за ФС');
		if (state.exclCaches) op.push('без каталогов кэша');
		if (num(state.limUp)) op.push('отдача до ' + num(state.limUp) + ' КиБ/с');
		if (num(state.limDown)) op.push('приём до ' + num(state.limDown) + ' КиБ/с');
		out.options = off(state, 'on_options', op.length > 0, op.join(' · '), 'Всё по умолчанию');

		var ret = retentionSummary(state);
		var retOn = isOn(state, 'on_retention');
		if (!retOn) out.retention = { on: false, disabled: true, text: 'Выключено: снимки не удаляются' };
		else if (!ret.length) out.retention = { on: true, warn: true, text: 'Правил нет: ничего не удаляется' };
		else out.retention = { on: true, text: ret.join(' · ') };

		var subset = String(state.subset || '').trim();
		out.check = off(state, 'on_check', true, subset ? 'читать ' + subset + ' данных' : 'только структура репозитория', 'Выключено');

		var sc = state.scriptsBefore + state.scriptsAfter;
		out.scripts = off(state, 'on_scripts', sc > 0, 'до: ' + state.scriptsBefore + ' · после: ' + state.scriptsAfter, 'Скриптов нет');

		var lim = [];
		lim.push(num(state.stall) ? 'без прогресса ' + num(state.stall) + ' мин' : 'без прогресса ' + STALL_DEFAULT + ' мин (по умолчанию)');
		lim.push(num(state.maxDur) ? 'не дольше ' + num(state.maxDur) + ' ч' : 'не дольше ' + MAX_DURATION_DEFAULT + ' ч (по умолчанию)');
		out.limits = { on: true, text: lim.join(' · ') };
		return out;
	}

	function off(state, key, hasValue, text, emptyText) {
		var enabled = isOn(state, key);
		if (!enabled) return { on: false, text: 'Выключено', disabled: true };
		if (!hasValue) return { on: false, text: emptyText, empty: true };
		return { on: true, text: text };
	}

	/* -------------------------------------------------------------- command */

	function quote(value) {
		return /[\s*?'"$\\[\]()]/.test(value) ? "'" + value.replace(/'/g, "'\\''") + "'" : value;
	}

	function buildSteps(state, os, scope) {
		var steps = [];
		var paths = lines(state.pathsAll).concat(lines(os === 'windows' ? state.pathsWin : os === 'macos' ? state.pathsMac : state.pathsLinux));
		var scripts = scope.querySelectorAll('[data-script]');
		var scriptsOn = isOn(state, 'on_scripts');
		function scriptSteps(phase) {
			if (!scriptsOn) return;
			for (var i = 0; i < scripts.length; i++) {
				if (scripts[i].getAttribute('data-phase') === phase) {
					steps.push({ kind: 'script', title: phase === 'before' ? 'Скрипт до копирования' : 'Скрипт после копирования', cmd: scripts[i].getAttribute('data-name') });
				}
			}
		}
		scriptSteps('before');
		var b = ['restic backup'];
		if (isOn(state, 'on_tags')) lines(state.tags).forEach(function (t) { b.push('--tag ' + quote(t)); });
		if (isOn(state, 'on_excludes')) {
			lines(state.excludes).forEach(function (t) { b.push('--exclude ' + quote(t)); });
			lines(state.excludeFiles).forEach(function (t) { b.push('--exclude-file ' + quote(t)); });
		}
		if (isOn(state, 'on_options')) {
			if (state.vss && os === 'windows') b.push('--use-fs-snapshot');
			if (state.oneFs) b.push('--one-file-system');
			if (state.exclCaches) b.push('--exclude-caches');
			if (num(state.limUp)) b.push('--limit-upload ' + num(state.limUp));
			if (num(state.limDown)) b.push('--limit-download ' + num(state.limDown));
		}
		paths.forEach(function (p) { b.push(quote(p)); });
		if (!paths.length) b.push('# пути не заданы');
		steps.push({ kind: 'backup', title: 'Резервное копирование', cmd: b.join(' \\\n  ') });
		scriptSteps('after');
		if (isOn(state, 'on_retention') && retentionHasRules(state)) {
			var f = ['restic forget'];
			var g = [];
			if (state.gbHost) g.push('host');
			if (state.gbPaths) g.push('paths');
			if (state.gbTags) g.push('tags');
			if (g.length) f.push('--group-by ' + g.join(','));
			RULES.forEach(function (rule) {
				if (num(state[rule.k])) f.push(rule.flag + ' ' + num(state[rule.k]));
			});
			if (String(state.keepWithin || '').trim()) f.push('--keep-within ' + String(state.keepWithin).trim());
			lines(state.keepTags).forEach(function (t) { f.push('--keep-tag ' + quote(t)); });
			lines(state.fHosts).forEach(function (t) { f.push('--host ' + quote(t)); });
			lines(state.fTags).forEach(function (t) { f.push('--tag ' + quote(t)); });
			lines(state.fPaths).forEach(function (t) { f.push('--path ' + quote(t)); });
			steps.push({ kind: 'forget', title: 'Удаление снимков', cmd: f.join(' \\\n  ') });
		}
		if (isOn(state, 'on_check')) {
			var subset = String(state.subset || '').trim();
			steps.push({ kind: 'check', title: 'Проверка', cmd: 'restic check' + (subset ? ' --read-data-subset ' + quote(subset) : '') });
		}
		return steps;
	}

	function drawPreview(scope, state) {
		var nodes = scope.querySelectorAll('[data-preview]');
		for (var i = 0; i < nodes.length; i++) {
			var osSel = nodes[i].querySelector('[data-preview-os]');
			var os = osSel ? osSel.value : 'linux';
			var target = nodes[i].querySelector('[data-preview-steps]');
			if (!target) continue;
			var steps = buildSteps(state, os, scope);
			var html = steps.map(function (step) {
				return '<div class="pm-step"><div class="pm-step-title">' + step.title + '</div><pre class="pm-cmd' + (step.kind === 'script' ? ' pm-cmd--script' : '') + '">' + esc(step.cmd) + '</pre></div>';
			}).join('');
			var lim = [];
			lim.push('нет прогресса ' + (num(state.stall) || STALL_DEFAULT) + ' мин');
			lim.push('всего ' + (num(state.maxDur) || MAX_DURATION_DEFAULT) + ' ч');
			if (lim.length) html += '<div class="pm-step-note">' + icon('clock', 'xs') + ' Остановка задания: ' + lim.join(', ') + '</div>';
			target.innerHTML = html;
		}
	}

	/* --------------------------------------------------------- page updating */

	function sentences(state) {
		var s = [];
		var unitText = {
			hourly: ['час', 'часа', 'часов', 'каждого часа'],
			daily: ['день', 'дня', 'дней', 'каждого дня'],
			weekly: ['неделя', 'недели', 'недель', 'каждой недели'],
			monthly: ['месяц', 'месяца', 'месяцев', 'каждого месяца'],
			yearly: ['год', 'года', 'лет', 'каждого года']
		};
		var one = { hourly: 'Последний час', daily: 'Последний день', weekly: 'Последняя неделя', monthly: 'Последний месяц', yearly: 'Последний год' };
		if (num(state.keepLast)) s.push('<li>Всегда остаются <b>' + num(state.keepLast) + '</b> ' + plural(num(state.keepLast), ['самый новый снимок', 'самых новых снимка', 'самых новых снимков']) + '.</li>');
		RULES.slice(1).forEach(function (rule) {
			var n = num(state[rule.k]);
			if (!n) return;
			var u = unitText[rule.unit];
			var head = n === 1 ? one[rule.unit] + ' с копиями' : 'Последние ' + n + ' ' + plural(n, [u[0], u[1], u[2]]) + ' с копиями';
			s.push('<li><b>' + head + ':</b> остаётся самый новый снимок ' + u[3] + '.</li>');
		});
		if (String(state.keepWithin || '').trim()) s.push('<li>Остаются все снимки не старше <b>' + esc(String(state.keepWithin).trim()) + '</b> от самого нового.</li>');
		if (lines(state.keepTags).length) s.push('<li>Снимки с тегами <b>' + lines(state.keepTags).map(esc).join(', ') + '</b> не удаляются никогда.</li>');
		if (!s.length) return '<li>Правил нет: ничего не удаляется, очистка не запускается.</li>';
		return s.join('');
	}

	function update(scope) {
		var state = read(scope);
		var sums = summarize(state);
		Object.keys(sums).forEach(function (key) {
			var nodes = scope.querySelectorAll('[data-sum="' + key + '"]');
			for (var i = 0; i < nodes.length; i++) nodes[i].textContent = sums[key].text;
			var dots = scope.querySelectorAll('[data-dot="' + key + '"]');
			for (var j = 0; j < dots.length; j++) {
				dots[j].className = 'pm-dot' + (sums[key].warn ? ' is-warn' : sums[key].on ? ' is-on' : ' is-off');
				dots[j].setAttribute('title', sums[key].warn ? 'Требует внимания' : sums[key].on ? 'Включено' : 'Выключено');
			}
			var bodies = scope.querySelectorAll('[data-sec-body="' + key + '"]');
			for (var k = 0; k < bodies.length; k++) bodies[k].classList.toggle('is-disabled', !!sums[key].disabled);
			var chips = scope.querySelectorAll('[data-chip="' + key + '"]');
			for (var m = 0; m < chips.length; m++) {
				chips[m].className = 'status ' + (sums[key].warn ? 'status--warning' : sums[key].on ? 'status--success' : 'status--neutral');
				chips[m].textContent = sums[key].warn ? 'Проверьте' : sums[key].on ? 'Включено' : 'Выключено';
			}
		});
		var plain = scope.querySelectorAll('[data-plain="retention"]');
		for (var p = 0; p < plain.length; p++) plain[p].innerHTML = isOn(state, 'on_retention') ? sentences(state) : '<li>Хранение выключено: снимки никогда не удаляются.</li>';
		var feats = scope.querySelectorAll('[data-feat]');
		for (var ft = 0; ft < feats.length; ft++) {
			var fs = sums[feats[ft].getAttribute('data-feat')];
			if (fs) feats[ft].classList.toggle('is-off', !!fs.disabled);
		}
		var listed = scope.querySelectorAll('[data-lines]');
		for (var ln = 0; ln < listed.length; ln++) {
			var items = lines(state[listed[ln].getAttribute('data-lines')]);
			listed[ln].innerHTML = items.length
				? items.map(function (item) { return '<span class="code">' + esc(item) + '</span>'; }).join(' ')
				: '<span class="pm-sub">нет</span>';
		}
		var live = scope.querySelectorAll('[data-live]');
		for (var l = 0; l < live.length; l++) {
			var field = live[l].getAttribute('data-live');
			live[l].textContent = state[field] || '';
		}
		var countOn = 0;
		var countAll = 0;
		['excludes', 'tags', 'options', 'retention', 'check', 'scripts'].forEach(function (key) {
			countAll++;
			if (sums[key].on) countOn++;
		});
		var counters = scope.querySelectorAll('[data-count-on]');
		for (var q = 0; q < counters.length; q++) counters[q].textContent = countOn + ' из ' + countAll;
		drawRetention(scope, state);
		drawPreview(scope, state);
		return state;
	}

	/* ---------------------------------------------------------- interactions */

	function setExpanded(button, open) {
		button.setAttribute('aria-expanded', open ? 'true' : 'false');
		var id = button.getAttribute('aria-controls');
		var body = id && document.getElementById(id);
		if (body) body.hidden = !open;
		var section = button.closest('[data-section]');
		if (section) section.classList.toggle('is-open', open);
	}

	function bindEvents(scope) {
		var refresh = function () {
			update(scope);
		};
		scope.addEventListener('input', function (event) {
			var chipInput = event.target.closest && event.target.closest('.pm-chipinput');
			if (chipInput && /[\s,]$/.test(chipInput.value)) commitChip(chipInput.closest('[data-chipbox]'));
			refresh();
		});
		scope.addEventListener('keydown', function (event) {
			var chipInput = event.target.closest && event.target.closest('.pm-chipinput');
			if (!chipInput) return;
			var box = chipInput.closest('[data-chipbox]');
			if (event.key === 'Enter') {
				event.preventDefault();
				commitChip(box);
			} else if (event.key === 'Backspace' && !chipInput.value) {
				var current = lines(box.querySelector('input[type="hidden"]').value);
				current.pop();
				setChips(box, current);
			}
		});
		scope.addEventListener('focusout', function (event) {
			var chipInput = event.target.closest && event.target.closest('.pm-chipinput');
			if (chipInput) commitChip(chipInput.closest('[data-chipbox]'));
		});
		scope.addEventListener('change', refresh);
		
		scope.addEventListener('click', function (event) {
			var toggle = event.target.closest('[data-sec-toggle]');
			if (toggle) {
				setExpanded(toggle, toggle.getAttribute('aria-expanded') !== 'true');
				return;
			}
			var script = event.target.closest('.pm-script-toggle');
			if (script) {
				var open = script.getAttribute('aria-expanded') !== 'true';
				script.setAttribute('aria-expanded', open ? 'true' : 'false');
				document.getElementById(script.getAttribute('aria-controls')).hidden = !open;
				script.closest('.pm-script').classList.toggle('is-open', open);
				return;
			}
			var chipRemove = event.target.closest('[data-chip-remove]');
			if (chipRemove) {
				var chipBox = chipRemove.closest('[data-chipbox]');
				var remaining = lines(chipBox.querySelector('input[type="hidden"]').value);
				remaining.splice(+chipRemove.getAttribute('data-chip-remove'), 1);
				setChips(chipBox, remaining);
				return;
			}
			var jump = event.target.closest('a[href^="#sec-"]');
			if (jump) {
				var jumpButton = scope.querySelector('[data-sec-toggle][aria-controls="' + jump.getAttribute('href').slice(1) + '"]');
				if (jumpButton) setExpanded(jumpButton, true);
				return;
			}
			var expand = event.target.closest('[data-expand-all]');
			if (expand) {
				var want = expand.getAttribute('data-expand-all') === 'open';
				var buttons = scope.querySelectorAll('[data-sec-toggle]');
				for (var i = 0; i < buttons.length; i++) setExpanded(buttons[i], want);
				return;
			}
			var preset = event.target.closest('[data-preset]');
			if (preset) {
				var data = PRESETS[preset.getAttribute('data-preset')];
				var map = ['keepLast', 'keepDaily', 'keepWeekly', 'keepMonthly', 'keepYearly'];
				var area = preset.closest('[data-scope]') || scope;
				map.forEach(function (k) {
					var input = area.querySelector('[data-k="' + k + '"]');
					if (input) input.value = data[k];
				});
				var rest = area.querySelectorAll('[data-k="keepHourly"], [data-k="keepWithin"]');
				for (var r = 0; r < rest.length; r++) rest[r].value = rest[r].getAttribute('data-k') === 'keepHourly' ? '0' : '';
				refresh();
				return;
			}
			if (event.target.closest('[data-script-add]')) {
				addScript(scope, { phase: 'before', name: 'Новый скрипт', template: '', interpreter: 'bash', timeout: '', policy: 'abort_backup', body: '', params: [] });
				refresh();
				return;
			}
			if (event.target.closest('[data-script-remove]')) {
				event.target.closest('[data-script]').remove();
				refresh();
				return;
			}
			if (event.target.closest('[data-param-remove]')) {
				event.target.closest('.pm-param').remove();
				return;
			}
			var copy = event.target.closest('[data-copy-preview]');
			if (copy) {
				var label = copy.querySelector('.btn-label');
				if (label) {
					var prev = label.textContent;
					label.textContent = 'Скопировано';
					window.setTimeout(function () { label.textContent = prev; }, 1200);
				}
			}
		});
		scope.addEventListener('input', function (event) {
			var nameInput = event.target.closest('[data-script-name]');
			if (nameInput) {
				var script = nameInput.closest('[data-script]');
				script.setAttribute('data-name', nameInput.value);
				script.querySelector('.pm-script-name').textContent = nameInput.value;
				refresh();
			}
		});
		scope.addEventListener('change', function (event) {
			if (event.target.closest('[data-preview-os]')) refresh();
			var sw = event.target.closest('[data-k^="on_"]');
			if (sw && sw.checked) {
				var holder = sw.closest('[data-section]');
				var opener = holder && holder.querySelector('[data-sec-toggle]');
				if (opener) setExpanded(opener, true);
			}
			var sel = event.target.closest('[data-script-phase-select]');
			if (sel) {
				var script = sel.closest('[data-script]');
				script.setAttribute('data-phase', sel.value);
				var target = scope.querySelector('[data-script-list="' + sel.value + '"]');
				if (target) target.appendChild(script);
				refresh();
			}
		});
	}

	/* ----------------------------------------------------------------- shell */

	function drawShell(active, crumb) {
		var tabs = [
			['overview', 'Обзор', 'empty-first-run.html'],
			['host', 'Хосты', 'hosts-ru.html'],
			['job', 'Задания', 'job.html'],
			['calendar', 'Планы', '#'],
			['repository', 'Репозитории', 'repositories.html'],
			['profile', 'Профили', 'profiles.html'],
			['restore', 'Восстановление', '#'],
			['audit', 'Журнал', 'audit.html'],
			['settings', 'Настройки', 'settings.html']
		];
		return (
			'<header class="topbar"><div class="topbar-inner">' +
			'<a class="brand" href="index.html"><span class="brandmark" aria-hidden="true"><svg viewBox="0 0 20 20"><path class="mark-solid" d="M10 2.5 2.5 6.4 10 10.3l7.5-3.9z" /><path d="M2.5 10.6 10 14.5l7.5-3.9" /></svg></span><span>ResticHub</span></a>' +
			'<span class="topbar-divider"></span>' +
			'<nav class="breadcrumb" aria-label="Путь"><span class="tenant-switch" style="cursor:default"><span>acme</span></span><span class="breadcrumb-sep">/</span>' +
			(crumb ? '<a href="profiles.html">Профили</a><span class="breadcrumb-sep">/</span><span aria-current="page">' + esc(crumb) + '</span>' : '<span aria-current="page">Профили</span>') + '</nav>' +
			'<div class="spacer"></div>' +
			'<button class="btn btn--icon" data-theme-toggle aria-label="Сменить тему">' + icon('moon') + '</button>' +
			'<span class="btn btn--ghost" style="cursor:default"><span class="avatar">АП</span><span>Анна Петрова</span></span>' +
			'</div></header>' +
			'<nav class="tabs" aria-label="Разделы">' +
			tabs.map(function (tab) {
				return '<a class="tab" href="' + tab[2] + '"' + (tab[0] === active ? ' aria-current="page"' : '') + '>' + icon(tab[0]) + ' ' + tab[1] + '</a>';
			}).join('') +
			'</nav>'
		);
	}

	/* ------------------------------------------------------------------ init */

	function init() {
		var shell = document.querySelector('[data-shell]');
		if (shell) shell.outerHTML = drawShell(shell.getAttribute('data-shell'), shell.getAttribute('data-crumb'));
		var scopes = document.querySelectorAll('[data-scope]');
		for (var i = 0; i < scopes.length; i++) {
			var mode = scopes[i].getAttribute('data-scope');
			var state = mode === 'new' ? EMPTY : SAMPLE;
			drawBlocks(scopes[i], state);
			var switches = scopes[i].querySelectorAll('[data-k^="on_"]');
			for (var s = 0; s < switches.length; s++) switches[s].checked = !!state[switches[s].getAttribute('data-k')];
			var boxes = scopes[i].querySelectorAll('[data-chipbox]');
			for (var cb = 0; cb < boxes.length; cb++) drawChips(boxes[cb]);
			bindEvents(scopes[i]);
		}
		var scopesNow = document.querySelectorAll('[data-scope]');
		for (var j = 0; j < scopesNow.length; j++) update(scopesNow[j]);
		var openers = document.querySelectorAll('[data-open-sec]');
		for (var o = 0; o < openers.length; o++) {
			var ids = openers[o].getAttribute('data-open-sec').split(',');
			ids.forEach(function (id) {
				var button = document.querySelector('[data-sec-toggle][aria-controls="sec-' + id + '"]');
				if (button) setExpanded(button, true);
			});
		}
		var hash = (window.location.hash || '').replace('#', '');
		if (hash) {
			var target = document.querySelector('[data-sec-toggle][aria-controls="sec-' + hash + '"]');
			if (target) setExpanded(target, true);
		}
	}

	window.PM = { update: update, read: read, summarize: summarize, plural: plural, SAMPLE: SAMPLE };
	if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
	else init();
})();
