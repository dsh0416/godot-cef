(function() {
    // Prevent re-initialization
    if (window.__imeHelperInitialized) return;
    window.__imeHelperInitialized = true;

    // Track IME active state
    window.__imeActive = false;

    // Check if element is an editable element
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

            // Handle contentEditable elements
            if (el.isContentEditable) {
                const sel = window.getSelection();
                if (sel && sel.rangeCount > 0) {
                    const range = sel.getRangeAt(0);
                    // Get the bounding rect of the collapsed cursor position
                    const rects = range.getClientRects();
                    if (rects.length > 0) {
                        rect = rects[rects.length - 1];
                    } else {
                        rect = range.getBoundingClientRect();
                    }
                }
            }
            // Handle input and textarea elements
            else if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
                // For input/textarea, we need to measure the caret position
                // Create a temporary span to measure text width up to cursor
                const pos = el.selectionStart || 0;
                const text = el.value.substring(0, pos);

                // Get element's computed style
                const style = window.getComputedStyle(el);

                // Create a measuring element
                const measurer = document.createElement('span');
                measurer.style.cssText = 'position:absolute;visibility:hidden;white-space:pre;' +
                    'font:' + style.font + ';' +
                    'font-size:' + style.fontSize + ';' +
                    'font-family:' + style.fontFamily + ';' +
                    'letter-spacing:' + style.letterSpacing + ';';
                measurer.textContent = text || '\u200b'; // Zero-width space if empty
                document.body.appendChild(measurer);

                const textWidth = measurer.offsetWidth;
                document.body.removeChild(measurer);

                // Get the element's bounding rect
                const elRect = el.getBoundingClientRect();
                const paddingLeft = parseFloat(style.paddingLeft) || 0;
                const borderLeft = parseFloat(style.borderLeftWidth) || 0;

                // Calculate caret position
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
                window.__sendImeCaretPosition(x, y, height);
            }
        } catch (e) {
            // Silently ignore errors
        }
    };

    // Activate IME tracking
    window.__activateImeTracking = function() {
        window.__imeActive = true;
        window.__reportCaretBounds();
    };

    // Deactivate IME tracking
    window.__deactivateImeTracking = function() {
        window.__imeActive = false;
    };

    // Listen for selection changes to update caret position
    document.addEventListener('selectionchange', function() {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            window.__reportCaretBounds();
        }
    });

    // Listen for input events (handles delete, paste, etc.)
    document.addEventListener('input', function(e) {
        if (window.__imeActive && isEditableElement(e.target)) {
            // Defer to allow the DOM to update
            setTimeout(function() {
                window.__reportCaretBounds();
            }, 0);
        }
    }, true);

    // Listen for keyup to catch arrow key navigation and other movements
    document.addEventListener('keyup', function(e) {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            const navKeys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 
                            'Home', 'End', 'PageUp', 'PageDown', 'Backspace', 'Delete'];
            if (navKeys.includes(e.key)) {
                window.__reportCaretBounds();
            }
        }
    }, true);

    // Listen for mouseup to catch click-to-reposition
    document.addEventListener('mouseup', function(e) {
        if (window.__imeActive && isEditableElement(document.activeElement)) {
            // Small delay to let selection settle
            setTimeout(function() {
                window.__reportCaretBounds();
            }, 10);
        }
    }, true);
})();
