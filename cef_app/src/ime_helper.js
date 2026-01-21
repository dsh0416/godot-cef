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

    // CSS properties to copy for accurate mirror element measurement
    const MIRROR_PROPERTIES = [
        'direction', 'boxSizing', 'width', 'height', 'overflowX', 'overflowY',
        'borderTopWidth', 'borderRightWidth', 'borderBottomWidth', 'borderLeftWidth',
        'borderStyle', 'paddingTop', 'paddingRight', 'paddingBottom', 'paddingLeft',
        'fontStyle', 'fontVariant', 'fontWeight', 'fontStretch', 'fontSize',
        'fontSizeAdjust', 'lineHeight', 'fontFamily', 'textAlign', 'textTransform',
        'textIndent', 'textDecoration', 'letterSpacing', 'wordSpacing',
        'tabSize', 'MozTabSize', 'whiteSpace', 'wordWrap', 'wordBreak'
    ];

    // Calculate caret coordinates for INPUT/TEXTAREA using mirror element technique
    function getCaretCoordinates(element, position) {
        const isInput = element.tagName === 'INPUT';
        const style = window.getComputedStyle(element);

        // Create mirror div
        const mirror = document.createElement('div');
        mirror.id = '__ime_mirror';
        document.body.appendChild(mirror);

        const mirrorStyle = mirror.style;
        mirrorStyle.position = 'absolute';
        mirrorStyle.visibility = 'hidden';
        mirrorStyle.whiteSpace = isInput ? 'nowrap' : 'pre-wrap';
        mirrorStyle.wordWrap = isInput ? 'normal' : 'break-word';

        // Copy computed styles to mirror
        MIRROR_PROPERTIES.forEach(function(prop) {
            if (prop === 'whiteSpace' || prop === 'wordWrap') return; // Already set above
            mirrorStyle[prop] = style[prop];
        });

        // For INPUT elements, disable height constraint to prevent clipping
        if (isInput) {
            mirrorStyle.height = 'auto';
            mirrorStyle.overflowY = 'visible';
        }

        // Position mirror at same location as element (for accurate font rendering)
        const elRect = element.getBoundingClientRect();
        mirrorStyle.left = elRect.left + window.scrollX + 'px';
        mirrorStyle.top = elRect.top + window.scrollY + 'px';

        // Copy text content up to caret position
        const textBeforeCaret = element.value.substring(0, position);
        
        // Use textContent for proper handling of special characters and line breaks
        // Replace spaces with non-breaking spaces to preserve trailing spaces
        mirror.textContent = textBeforeCaret;
        
        // If text ends with newline, add a placeholder to ensure line height is measured
        if (textBeforeCaret.endsWith('\n')) {
            mirror.textContent += '\u200b'; // Zero-width space
        }

        // Create caret marker span
        const marker = document.createElement('span');
        marker.textContent = '\u200b'; // Zero-width space as marker
        mirror.appendChild(marker);

        // Get marker position
        const markerRect = marker.getBoundingClientRect();
        const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.2;

        // Calculate coordinates relative to viewport, accounting for scroll within element
        const coordinates = {
            x: markerRect.left - element.scrollLeft,
            y: markerRect.top - element.scrollTop,
            height: lineHeight
        };

        // Clean up
        document.body.removeChild(mirror);

        return coordinates;
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
                rect = getCaretCoordinates(el, pos);
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
