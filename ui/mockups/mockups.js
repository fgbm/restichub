/*
 * ResticHub mockups: the small behaviour the static pages need.
 * Theme toggle, underline tabs, menus, dialogs, side panels, toasts and copy fields.
 * No other script runs on these pages, and no page carries an inline script.
 */
(function () {
	'use strict';

	var STORE_KEY = 'rh-mock-theme';
	var media = window.matchMedia('(prefers-color-scheme: dark)');

	function storedTheme() {
		try {
			return window.localStorage.getItem(STORE_KEY);
		} catch (error) {
			return null;
		}
	}

	function storeTheme(value) {
		try {
			if (value) window.localStorage.setItem(STORE_KEY, value);
			else window.localStorage.removeItem(STORE_KEY);
		} catch (error) {
			/* Private mode without storage. The theme applies for this page only. */
		}
	}

	function systemTheme() {
		return media.matches ? 'dark' : 'light';
	}

	function applyTheme() {
		var stored = storedTheme();
		var effective = stored === 'light' || stored === 'dark' ? stored : systemTheme();
		document.documentElement.setAttribute('data-theme', effective);
		document.documentElement.setAttribute('data-theme-mode', stored || 'system');
		updateThemeButtons(effective, stored || 'system');
	}

	function updateThemeButtons(effective, mode) {
		var toggles = document.querySelectorAll('[data-theme-toggle]');
		for (var i = 0; i < toggles.length; i++) {
			var next = effective === 'dark' ? 'light' : 'dark';
			toggles[i].setAttribute('aria-label', 'Switch to the ' + next + ' theme');
			toggles[i].setAttribute('title', 'Switch to the ' + next + ' theme');
			var use = toggles[i].querySelector('use');
			if (use) use.setAttribute('href', 'icons.svg#i-' + (effective === 'dark' ? 'sun' : 'moon'));
		}
		var setters = document.querySelectorAll('[data-theme-set]');
		for (var j = 0; j < setters.length; j++) {
			setters[j].setAttribute('aria-checked', setters[j].getAttribute('data-theme-set') === mode ? 'true' : 'false');
		}
	}

	function setTheme(value) {
		storeTheme(value === 'system' ? null : value);
		applyTheme();
	}

	function initTheme() {
		applyTheme();
		media.addEventListener('change', function () {
			if (!storedTheme()) applyTheme();
		});
		document.addEventListener('click', function (event) {
			var toggle = event.target.closest('[data-theme-toggle]');
			if (toggle) {
				var current = document.documentElement.getAttribute('data-theme');
				setTheme(current === 'dark' ? 'light' : 'dark');
				return;
			}
			var setter = event.target.closest('[data-theme-set]');
			if (setter) setTheme(setter.getAttribute('data-theme-set'));
		});
	}

	/* Underline tabs: one active tab, the matching panel shown. */
	function activateTab(tab) {
		var list = tab.closest('[role="tablist"]');
		if (!list) return;
		var tabs = list.querySelectorAll('[role="tab"]');
		for (var i = 0; i < tabs.length; i++) {
			var selected = tabs[i] === tab;
			tabs[i].setAttribute('aria-selected', selected ? 'true' : 'false');
			tabs[i].setAttribute('tabindex', selected ? '0' : '-1');
			var panelId = tabs[i].getAttribute('aria-controls');
			if (panelId) {
				var panel = document.getElementById(panelId);
				if (panel) panel.hidden = !selected;
			}
		}
	}

	function initTabs() {
		document.addEventListener('click', function (event) {
			var tab = event.target.closest('[role="tab"]');
			if (tab && tab.closest('[role="tablist"]')) activateTab(tab);
		});
		document.addEventListener('keydown', function (event) {
			var tab = event.target.closest('[role="tab"]');
			if (!tab || (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft')) return;
			var tabs = Array.prototype.slice.call(tab.closest('[role="tablist"]').querySelectorAll('[role="tab"]'));
			var delta = event.key === 'ArrowRight' ? 1 : -1;
			var next = tabs[(tabs.indexOf(tab) + delta + tabs.length) % tabs.length];
			event.preventDefault();
			next.focus();
			activateTab(next);
		});
	}

	/* Menus (action menus and the user menu). */
	function closeMenus(except) {
		var open = document.querySelectorAll('[data-menu][data-open="true"]');
		for (var i = 0; i < open.length; i++) {
			if (open[i] === except) continue;
			open[i].setAttribute('data-open', 'false');
			open[i].hidden = true;
			var owner = document.querySelector('[data-menu-toggle="' + open[i].id + '"]');
			if (owner) owner.setAttribute('aria-expanded', 'false');
		}
	}

	function initMenus() {
		document.addEventListener('click', function (event) {
			var toggle = event.target.closest('[data-menu-toggle]');
			if (toggle) {
				var menu = document.getElementById(toggle.getAttribute('data-menu-toggle'));
				if (!menu) return;
				var open = menu.getAttribute('data-open') === 'true';
				closeMenus(open ? null : menu);
				menu.hidden = open;
				menu.setAttribute('data-open', open ? 'false' : 'true');
				toggle.setAttribute('aria-expanded', open ? 'false' : 'true');
				return;
			}
			if (!event.target.closest('[data-menu]')) closeMenus(null);
		});
		document.addEventListener('keydown', function (event) {
			if (event.key === 'Escape') closeMenus(null);
		});
	}

	/* Dialogs and side panels: native <dialog> handles Escape and the focus trap. */
	function initDialogs() {
		document.addEventListener('click', function (event) {
			var opener = event.target.closest('[data-open-dialog]');
			if (opener) {
				var dialog = document.getElementById(opener.getAttribute('data-open-dialog'));
				if (dialog && dialog.showModal) dialog.showModal();
				return;
			}
			var closer = event.target.closest('[data-close-dialog]');
			if (closer) {
				var owner = closer.closest('dialog');
				if (owner) owner.close();
				return;
			}
			var dialogEl = event.target;
			/* A click on the backdrop closes the dialog. */
			if (dialogEl.tagName === 'DIALOG') {
				var rect = dialogEl.getBoundingClientRect();
				var outside =
					event.clientX < rect.left ||
					event.clientX > rect.right ||
					event.clientY < rect.top ||
					event.clientY > rect.bottom;
				if (outside) dialogEl.close();
			}
			var panel = event.target.closest('[data-close-panel]');
			if (panel) {
				var aside = panel.closest('[data-panel]');
				if (aside) aside.removeAttribute('data-open');
			}
		});
		var openers = document.querySelectorAll('[data-open-panel]');
		for (var i = 0; i < openers.length; i++) {
			openers[i].addEventListener('click', function () {
				var aside = document.getElementById(this.getAttribute('data-open-panel'));
				if (aside) aside.setAttribute('data-open', 'true');
			});
		}
	}

	/* Typed confirmation: the destructive button stays disabled until the text matches. */
	function initTypedConfirm() {
		var inputs = document.querySelectorAll('[data-typed-confirm]');
		for (var i = 0; i < inputs.length; i++) {
			inputs[i].addEventListener('input', function () {
				var target = document.getElementById(this.getAttribute('data-typed-confirm'));
				if (target) target.disabled = this.value.trim() !== this.getAttribute('data-typed-value');
			});
		}
	}

	function initToasts() {
		document.addEventListener('click', function (event) {
			if (event.target.closest('[data-toast-close]')) {
				var toast = event.target.closest('.toast');
				if (toast) toast.remove();
			}
			var show = event.target.closest('[data-toast-show]');
			if (show) {
				var toast = document.getElementById(show.getAttribute('data-toast-show'));
				if (toast) toast.removeAttribute('hidden');
			}
		});
	}

	function initCopy() {
		document.addEventListener('click', function (event) {
			var button = event.target.closest('[data-copy]');
			if (!button) return;
			var field = document.getElementById(button.getAttribute('data-copy'));
			if (!field) return;
			var text = field.value !== undefined ? field.value : field.textContent.trim();
			if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(text);
			var label = button.querySelector('.btn-label') || button;
			var previous = label.textContent;
			label.textContent = 'Copied';
			window.setTimeout(function () {
				label.textContent = previous;
			}, 1200);
		});
	}

	function initCombobox() {
		document.addEventListener('click', function (event) {
			var option = event.target.closest('[data-combobox-option]');
			if (!option) return;
			var root = option.closest('[data-combobox]');
			var list = root.querySelector('[data-combobox-list]');
			var label = root.querySelector('[data-combobox-label]');
			if (label) label.textContent = option.textContent.trim();
			if (list) list.hidden = true;
		});
	}

	function initIndeterminate() {
		var boxes = document.querySelectorAll('[data-indeterminate]');
		for (var i = 0; i < boxes.length; i++) boxes[i].indeterminate = true;
	}

	/* The mockups have no control plane: a form submit does nothing. */
	function initForms() {
		document.addEventListener('submit', function (event) {
			event.preventDefault();
		});
	}

	function init() {
		initTheme();
		initIndeterminate();
		initForms();
		initTabs();
		initMenus();
		initDialogs();
		initTypedConfirm();
		initToasts();
		initCopy();
		initCombobox();
	}

	if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
	else init();
})();
