import { Navigate } from 'react-router-dom';

import Translate from '../pages/Translate';
import Service from '../pages/Service';
import Wordbook from '../pages/Wordbook';
import About from '../pages/About';

const routes = [
    {
        path: '/translate',
        element: <Translate />,
    },
    {
        path: '/service',
        element: <Service />,
    },
    {
        path: '/wordbook',
        element: <Wordbook />,
    },
    {
        path: '/about',
        element: <About />,
    },
    {
        path: '/',
        element: <Navigate to='/translate' />,
    },
];

export default routes;
