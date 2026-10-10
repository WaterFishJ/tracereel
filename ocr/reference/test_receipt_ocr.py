import unittest
import numpy as np
from receipt_ocr import rectify, grid_from_boxes


class ReceiptGeometryTests(unittest.TestCase):
    def test_perspective_and_rotation(self):
        image = np.full((200,100,3),255,np.uint8)
        corners = [[.1,.1],[.9,.2],[.8,.9],[.2,.8]]
        upright = rectify(image,corners)
        rotated = rectify(image,corners,1)
        self.assertEqual(upright.shape[:2], rotated.shape[1::-1])

    def test_crossed_quad_rejected(self):
        with self.assertRaises(ValueError):
            rectify(np.zeros((200,100,3),np.uint8), [[0,0],[1,1],[1,0],[0,1]])

    def test_missing_cells_and_order_retained(self):
        def box(x,y):
            return [[x,y],[x+25,y],[x+25,y+12],[x,y+12]]
        table = grid_from_boxes([(box(10,10),'1',.99),(box(110,10),'120',.99),
                                 (box(210,10),'00:01:21',.99),(box(10,35),'2',.99),
                                 (box(210,35),'00:01:24',.99)], [0,90,190,300])
        self.assertEqual(table['rows'][1]['cells'], ['2','','00:01:24'])


if __name__ == '__main__':
    unittest.main()
