(function() {
    if (window.__imeHelperInitialized) return;
    window.__imeHelperInitialized = true;

    window.__imeActive = false;

    function isEditableElement(el) {
        if (!el) return false;
        if (el.isContentEditable) return true;
        if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') return true;
        return false;
    }

    window.__reportCaretBounds = function() {
        try {
            const el = document.activeElement;
            if (!el || !isEditableElement(el)) return;

            let rect = null;

            if (el.isContentEditable) {
                const sel = window.getSelection();
                if (sel && sel.rangeCount > 0) {
                    const range = sel.getRangeAt(0);
                    const rects = range.getClientRects();
                    if (rects.length > 0) {
                        rect = rects[rects.length - 1];
                    } else {
                        rect = range.getBoundingClientRect();
                    }
                }
            } else if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
                const pos = el.selectionStart || 0;
                const text = el.value.substring(0, pos);
                const style = window.getComputedStyle(el);

                const measurer = document.createElement('span');
                measurer.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;' +
                    'font:' + style.font + ';' +
                    'font-size:' + style.fontSize + ';' +
                    'font-family:' + style.fontFamily + ';' +
                    'letter-spacing:' + style.letterSpacing + ';';
                measurer.textContent = text || '\u200b';
                document.body.appendChild(measurer);

                const textWidth = measurer.offsetWidth;
                document.body.removeChild(measurer);

                const elRect = el.getBoundingClientRect();
                const paddingLeft = parseFloat(style.paddingLeft) || 0;
                const borderLeft = parseFloat(style.borderLeftWidth) || 0;
                const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.2;

                rect = {
                    x: elRect.left + paddingLeft + borderLeft + textWidth,
                    y: elRect.top + parseFloat(style.paddingTop || 0) + parseFloat(style.borderTopWidth || 0),
                    height: lineHeight
                };
            }

            if (rect && (rect.width !== undefined || rect.x !== undefined)) {
                const x = Math.round(rect.x || rect.left || 0);
                const y = Math.round(rect.y || rect.top || 0);
                const height = Math.round(rect.height || 20);
                if (typeof window.__sendImeCaretPosition === 'function') {
                    window.__sendImeCaretPosition(x, y, height);
                }
            }
        } catch (e) {
            if (typeof console !== 'undefined' && typeof console.error === 'function') {
                console.error('IME helper: error while reporting caret bounds:', e);
            }
        }
    };

    window.__activateImeTracking = function() {
        window.__imeActive = true;
        window.__reportCaretBounds();
    };

    window.__deactivateImeTracking = function() {
        window.__imeActive = false;
    };

    document.addEventListener('selectionchange', function() {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            window.__reportCaretBounds();
        }
    });

    document.addEventListener('input', function(e) {
        if (window.__imeActive && isEditableElement(e.target)) {
            setTimeout(function() { window.__reportCaretBounds(); }, 0);
        }
    }, true);

    document.addEventListener('keyup', function(e) {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            const navKeys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 
                            'Home', 'End', 'PageUp', 'PageDown', 'Backspace', 'Delete'];
            if (navKeys.includes(e.key)) {
                window.__reportCaretBounds();
            }
        }
    }, true);

    document.addEventListener('mouseup', function(e) {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            setTimeout(function() { window.__reportCaretBounds(); }, 10);
        }
    }, true);
})();
